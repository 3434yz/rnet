use crate::acceptor::Acceptor;
use crate::balancer::Balancer;
use crate::event_loop::{EventLoop, EventLoopHandle};
use crate::handler::EventHandler;
use crate::listener::Listener;
use crate::options::{Options, get_core_ids};
use crate::socket_addr::NetworkAddress;
use crate::worker::WorkerPool;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::{io, thread};

pub struct EngineBuilder {
    addrs: Option<Vec<NetworkAddress>>,
    lb_policy: Option<Balancer>,
}

impl EngineBuilder {
    pub fn builder() -> Self {
        Self {
            lb_policy: None,
            addrs: None,
        }
    }

    pub fn build<H, F>(self, options: Options, handler_factory: F) -> (Engine<H>, EngineHandler)
    where
        H: EventHandler,
        F: FnOnce(EngineHandler) -> H,
    {
        let options = Arc::new(options);
        let engine_handler = EngineHandler {};
        let handler = handler_factory(engine_handler.clone());

        let lb_policy = if options.reuse_port {
            None
        } else {
            Some(Balancer::new(options.lb))
        };

        let engine = Engine {
            address: self.addrs.unwrap(),
            options: options.clone(),
            handler: Some(Arc::new(handler)),
            lb_policy,
        };
        (engine, engine_handler)
    }

    pub fn address(mut self, addrs: Vec<NetworkAddress>) -> Self {
        self.addrs = Some(addrs);
        self
    }

    pub fn load_balancer(mut self, lb: Balancer) -> Self {
        self.lb_policy = Some(lb);
        self
    }
}

#[derive(Clone, Copy)]
pub struct EngineHandler {}

impl EngineHandler {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

pub struct Engine<H: EventHandler> {
    handler: Option<Arc<H>>,
    address: Vec<NetworkAddress>,
    options: Arc<Options>,
    lb_policy: Option<Balancer>,
}

impl<H> Engine<H>
where
    H: EventHandler,
{
    pub fn run(&mut self) -> io::Result<()> {
        let cores = get_core_ids(Some(self.options.num_event_loop));

        println!("Engine starting with {} IO workers...", cores.len());

        if self.options.reuse_port {
            self.run_reuse_port(cores)
        } else {
            self.run_user_lb(cores)
        }
    }

    fn run_user_lb(&mut self, cores: Vec<core_affinity::CoreId>) -> io::Result<()> {
        println!("Engine runing user lb");

        let listeners = self
            .address
            .iter()
            .map(|addr| Listener::bind(addr.clone(), self.options.clone()))
            .collect::<io::Result<Vec<Listener>>>()?;

        let mut workers = Vec::new();
        let mut threads = Vec::new();

        let handler = self.handler.as_ref().unwrap().clone();

        for (loop_id, core_id) in cores.into_iter().enumerate() {
            let (urgent_sender, urgent_receiver) = crossbeam::channel::unbounded();
            let (common_sender, common_receiver) = crossbeam::channel::unbounded();
            let conn_count = Arc::new(AtomicUsize::new(0));

            let mut event_loop = EventLoop::new(
                loop_id as u8,
                self.options.clone(),
                handler.clone(),
                urgent_sender,
                urgent_receiver,
                common_sender,
                common_receiver,
                conn_count,
            )?;

            workers.push(event_loop.handle().clone());

            let t = thread::spawn(move || {
                if !core_affinity::set_for_current(core_id) {
                    eprintln!("EventLoop {} failed to pin to core", loop_id);
                }
                if let Err(e) = event_loop.run() {
                    eprintln!("EventLoop {} failed: {}", loop_id, e);
                }
            });
            threads.push(t);
        }

        // 构建 Registry，用于 WorkerPool 回传消息
        let registry = workers.clone();

        let balancer = self.lb_policy.take().unwrap();
        let mut acceptor = Acceptor::new(listeners, workers, balancer)?;

        println!("Starting Acceptor thread...");
        let acceptor_thread = thread::spawn(move || {
            if let Err(e) = acceptor.run() {
                eprintln!("Acceptor failed: {}", e);
            }
        });
        threads.push(acceptor_thread);

        let worker_pool = WorkerPool::new(registry);
        worker_pool.run(4);

        for t in threads {
            t.join().unwrap();
        }

        Ok(())
    }

    fn run_reuse_port(&mut self, cores: Vec<core_affinity::CoreId>) -> io::Result<()> {
        println!("Engine runing kernel lb");

        let mut threads = Vec::new();

        let mut registry = Vec::new();
        let handler = self.handler.as_ref().expect("Handler is none").clone();
        struct PendingLoop<H: EventHandler> {
            el: EventLoop<H>,
            core: core_affinity::CoreId,
            handle: Arc<EventLoopHandle>,
        }

        let mut pending = Vec::new();

        for (loop_id, core_id) in cores.into_iter().enumerate() {
            let (urgent_sender, urgent_receiver) = crossbeam::channel::unbounded();
            let (common_sender, common_receiver) = crossbeam::channel::unbounded();

            let listeners = self
                .address
                .iter()
                .map(|addr| Listener::bind(addr.clone(), self.options.clone()))
                .collect::<io::Result<Vec<_>>>()?;
            let conn_count = Arc::new(AtomicUsize::new(0));

            let event_loop = EventLoop::new(
                loop_id as u8,
                self.options.clone(),
                handler.clone(),
                urgent_sender,
                urgent_receiver,
                common_sender,
                common_receiver,
                conn_count,
            )?
            .listener(listeners)
            .build()?;

            let handle = event_loop.handle().clone();
            pending.push(PendingLoop {
                el: event_loop,
                core: core_id,
                handle: handle,
            });
        }

        for p in &pending {
            registry.push(p.handle.clone());
        }

        println!("Starting 4 Business Workers...");
        let worker_pool = WorkerPool::new(registry);
        worker_pool.run(4);

        for p in pending {
            let mut el = p.el;
            let core = p.core;
            let idx = el.loop_id;
            let t = thread::spawn(move || {
                if !core_affinity::set_for_current(core) {
                    eprintln!("EventLoop {} failed to pin", idx);
                }
                if let Err(e) = el.run() {
                    eprintln!("EventLoop failed: {}", e);
                }
            });
            threads.push(t);
        }

        for t in threads {
            t.join().unwrap();
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::connection::Connection;
    use crate::engine::{EngineBuilder, EngineHandler};
    use crate::handler::{Action, EventHandler};
    use crate::options::Options;
    use bytes::BytesMut;

    #[derive(Clone, Copy)]
    struct GameServer {
        engine: Option<EngineHandler>,
    }

    impl EventHandler for GameServer {
        fn on_open(&self, _conn: &mut Connection) -> Action {
            println!("New Connect");
            Action::None
        }

        fn on_traffic(&self, _conn: &mut Connection, _cache: &mut BytesMut) -> Action {
            Action::None
        }

        fn on_close(&self, _ctx: &mut Connection) -> Action {
            println!("Close Connect");
            Action::None
        }
    }
    #[test]
    fn run_reuse_port() {
        let mut options = Options::builder()
            .reuse_port(true)
            .reuse_addr(true)
            .num_event_loop(8)
            .build();

        let addrs = vec!["tcp://127.0.0.1:9000"];
        let net_socket_addrs = options.normalize(&addrs).unwrap();

        let (mut engine, _handler_copy) =
            EngineBuilder::builder()
                .address(net_socket_addrs)
                .build(options, |engine_handler| GameServer {
                    engine: Some(engine_handler),
                });

        engine.run().expect("run failed");
    }

    #[test]
    fn run_user_lb() {
        let mut options = Options::builder()
            .lb(crate::options::LoadBalancing::RoundRobin)
            .reuse_addr(true)
            .num_event_loop(8)
            .build();

        let addrs = vec![
            "tcp://127.0.0.1:9000",
            "tcp://127.0.0.1:9001",
            "tcp://127.0.0.1:9002",
        ];
        let net_socket_addrs = options.normalize(&addrs).unwrap();

        let (mut engine, _handler_copy) =
            EngineBuilder::builder()
                .address(net_socket_addrs)
                .build(options, |engine_handler| GameServer {
                    engine: Some(engine_handler),
                });

        engine.run().expect("run failed");
    }
}
