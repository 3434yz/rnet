use crate::handler::EventHandler;
use crate::listener::Listener;
use crate::options::Options;
use crate::socket_addr::NetworkAddress;
use crate::event_loop::EventLoop;
use crate::worker::start_worker;

use std::sync::Arc;
use std::{io, thread};

pub struct EngineBuilder {
    addrs: Option<Vec<NetworkAddress>>,
}

impl EngineBuilder {
    pub fn builder() -> Self {
        Self { addrs: None }
    }

    pub fn build<H: EventHandler>(
        self,
        options: Options,
        handler: H,
    ) -> (Engine<H>, EngineHandler) {
        let options = Arc::new(options);
        let engine = Engine {
            address: self.addrs.unwrap(),
            options: options.clone(),
            handler: Arc::new(handler),
        };
        let engine_handler = EngineHandler {};

        (
            engine,
            engine_handler,
        )
    }

    pub fn address(mut self, addrs: Vec<NetworkAddress>) -> Self {
        self.addrs = Some(addrs);
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

struct EventLoopInitCtx<H: EventHandler> {
    event_loop: EventLoop<H>,
    core_id: core_affinity::CoreId,
}

pub struct Engine<H: EventHandler> {
    handler: Arc<H>,
    address: Vec<NetworkAddress>,
    options: Arc<Options>,
}

impl<H> Engine<H>
where
    H: EventHandler,
{
    pub fn run(&mut self) -> io::Result<()> {
        let core_ids = core_affinity::get_core_ids().unwrap();
        let p_cores = 8;
        let worker_cores: Vec<_> = core_ids.into_iter().take(p_cores * 2).step_by(2).collect();
        println!(
            "Starting {} workers (EventLoop Wrapped)...",
            worker_cores.len()
        );
        let mut event_loop_handles = Vec::new();
        let (req_tx, req_rx) = crossbeam::channel::unbounded();
        let mut event_loops = Vec::new();
        let mut pending_event_loops = Vec::new();

        for (i, core_id) in worker_cores.into_iter().enumerate() {
            let (resp_tx, resp_rx) = crossbeam::channel::unbounded();
            let listener = Listener::bind(self.address[0].clone(), self.options.clone())?;
            let handler = self.handler.clone();
            let event_loop = EventLoop::new(i, listener, handler, req_tx.clone(), resp_rx)?;
            event_loops.push((resp_tx, event_loop.get_waker()));
            pending_event_loops.push(EventLoopInitCtx{
                event_loop,
                core_id,
            });
        }

        println!("Starting 4 Workers...");
        for i in 0..4 {
            let req_rx = req_rx.clone();
            let registry = event_loops.clone();
            start_worker(i, req_rx, registry, |job| -> Vec<u8> {
                thread::sleep(std::time::Duration::from_secs(5));
                job.into()
            });
        }

        for (i, ctx) in pending_event_loops.into_iter().enumerate() {
            let mut event_loop = ctx.event_loop;
            let core_id = ctx.core_id;

            let handle = thread::spawn(move || {
                if !core_affinity::set_for_current(core_id) {
                    eprintln!("EventLoop {} failed to pin to core", i);
                }
                if let Err(e) = event_loop.run() {
                    eprintln!("EventLoop {} failed: {}", i, e);
                }
            });
            event_loop_handles.push(handle);
        }

        drop(event_loops);

        for handle in event_loop_handles {
            handle.join().unwrap();
        }

        Ok(())
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
    struct MyHandler {
        engine: Option<EngineHandler>,
    }

    impl EventHandler for MyHandler {
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
    fn run() {
        let mut options = Options::builder()
            .reuse_port(true)
            .reuse_addr(true)
            .multicore(true)
            .build();

        let addrs = vec!["tcp://127.0.0.1:9000"];
        let net_socket_addrs = options.normalize(&addrs).unwrap();
        let my_handler = MyHandler{
            engine:None
        };

        let (mut engine, _handler) = EngineBuilder::builder()
            .address(net_socket_addrs)
            .build::<MyHandler>(options, my_handler);

        engine.run().expect("run failed");
    }
}
