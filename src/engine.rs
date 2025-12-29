use crate::acceptor::Acceptor;
use crate::balancer::Balancer;
use crate::event_loop::{ConnectionInitializer, EventLoop, EventLoopHandle};
use crate::handler::EventHandler;
use crate::listener::Listener;
use crate::options::{Options, get_core_ids};
use crate::socket_addr::NetworkAddress;
use crate::worker::start_worker;

use std::cmp::min;
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
        let core_ids = get_core_ids(Some(self.options.num_event_loop));

        println!("Engine starting with {} IO workers...", core_ids.len());

        if self.options.reuse_port {
            self.run_reuse_port(core_ids)
        } else {
            self.run_user_lb(core_ids)
        }
    }

    fn run_user_lb(&mut self, worker_cores: Vec<core_affinity::CoreId>) -> io::Result<()> {
        println!("Engine runing user lb");
        let listener = Listener::bind(self.address[0].clone(), self.options.clone())?;
        let mut handles = Vec::new();
        let mut threads = Vec::new();

        let (req_tx, req_rx) = crossbeam::channel::unbounded();
        let handler = self.handler.as_ref().unwrap().clone();

        for (idx, core_id) in worker_cores.into_iter().enumerate() {
            let (conn_tx, conn_rx) = crossbeam::channel::bounded::<ConnectionInitializer>(1024);
            let (resp_tx, resp_rx) = crossbeam::channel::unbounded();
            let conn_count = Arc::new(AtomicUsize::new(0));

            let mut event_loop = EventLoop::new(
                idx,
                None,
                handler.clone(),
                Some(conn_rx),
                req_tx.clone(),
                resp_rx,
                Some(conn_count.clone()),
            )?;

            let handle = EventLoopHandle::new(idx, conn_tx, event_loop.get_waker(), conn_count);
            handles.push(handle);

            // 2.6 启动 IO 线程
            let t = thread::spawn(move || {
                if !core_affinity::set_for_current(core_id) {
                    eprintln!("EventLoop {} failed to pin to core", idx);
                }
                if let Err(e) = event_loop.run() {
                    eprintln!("EventLoop {} failed: {}", idx, e);
                }
            });
            threads.push(t);
        }

        let balancer = self.lb_policy.take().unwrap();
        let mut acceptor = Acceptor::new(listener, handles, balancer)?;

        println!("Starting Acceptor thread...");
        let acceptor_thread = thread::spawn(move || {
            if let Err(e) = acceptor.run() {
                eprintln!("Acceptor failed: {}", e);
            }
        });
        threads.push(acceptor_thread);

        self.start_business_workers(req_rx)?;

        for t in threads {
            t.join().unwrap();
        }

        Ok(())
    }

    fn run_reuse_port(&mut self, worker_cores: Vec<core_affinity::CoreId>) -> io::Result<()> {
        println!("Engine runing kernel lb");
        let (req_tx, req_rx) = crossbeam::channel::unbounded();
        let mut threads = Vec::new();

        let mut registry = Vec::new();
        let handler = self.handler.as_ref().unwrap().clone();
        struct PendingLoop<H: EventHandler> {
            el: EventLoop<H>,
            core: core_affinity::CoreId,
            resp_tx: crossbeam::channel::Sender<crate::command::Response>,
            waker: Arc<mio::Waker>,
        }

        let mut pending = Vec::new();

        for (idx, core_id) in worker_cores.into_iter().enumerate() {
            let (resp_tx, resp_rx) = crossbeam::channel::unbounded();

            let listener = Listener::bind(self.address[0].clone(), self.options.clone())?;

            let event_loop = EventLoop::new(
                idx,
                Some(listener),
                handler.clone(),
                None,
                req_tx.clone(),
                resp_rx,
                None,
            )?;

            let waker = event_loop.get_waker();
            pending.push(PendingLoop {
                el: event_loop,
                core: core_id,
                resp_tx,
                waker,
            });
        }

        for p in &pending {
            registry.push((p.resp_tx.clone(), p.waker.clone()));
        }

        println!("Starting 4 Business Workers...");
        for i in 0..4 {
            let rx = req_rx.clone();
            let reg = registry.clone();
            start_worker(i, rx, reg, |job| -> Vec<u8> {
                thread::sleep(std::time::Duration::from_secs(5));
                job.into()
            });
        }

        for p in pending {
            let mut el = p.el;
            let core = p.core;
            let idx = el.idx;
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

    fn start_business_workers(
        &self,
        req_rx: crossbeam::channel::Receiver<crate::command::Request<H::Job>>,
    ) -> io::Result<()> {
        Ok(())
    }

    pub fn handler(&mut self, handler: Arc<H>) {
        self.handler = Some(handler.clone());
    }
}

#[cfg(test)]
mod tests {
    use crate::connection::Connection;
    use crate::engine::{EngineBuilder, EngineHandler};
    use crate::handler::{Action, Context, EventHandler};
    use crate::options::Options;
    use bytes::BytesMut;

    #[derive(Clone, Copy)]
    struct GameServer {
        engine: Option<EngineHandler>,
    }

    impl EventHandler for GameServer {
        type Job = Vec<u8>;

        fn on_open(&self, _conn: &mut Connection) -> Action<Self::Job> {
            println!("New Connect");
            Action::None
        }

        fn on_traffic(&self, _conn: &mut Connection, _cache: &mut BytesMut) -> Action<Self::Job> {
            Action::None
        }

        fn on_close(&self, _ctx: &mut Context) -> Action<Self::Job> {
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
}
