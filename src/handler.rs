use crate::connection::Connection;
use crate::engine::EngineHandler;
use crate::options::Options;
use slab::Slab;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Close,
    Shutdown,
}

pub struct TickContext<'a> {
    pub(crate) connections: &'a mut Slab<Connection>,
}

impl<'a> TickContext<'a> {
    pub fn connections(&mut self) -> impl Iterator<Item = &mut Connection> {
        self.connections.iter_mut().map(|(_, c)| c)
    }
}

pub trait EventHandler: Send + Sync + 'static {
    fn init(engine: Arc<EngineHandler>, options: Arc<Options>) -> (Self, Action)
    where
        Self: Sized;

    fn on_open(&self, _conn: &mut Connection) -> Action {
        Action::None
    }

    fn on_traffic(&self, _conn: &mut Connection) -> Action {
        Action::None
    }

    fn on_close(&self, _conn: &mut Connection) -> Action {
        Action::None
    }

    fn on_tick(&self, _ctx: &mut TickContext) -> (std::time::Duration, Action) {
        (std::time::Duration::from_secs(0), Action::None)
    }

    fn on_shutdon(&self) {}
}
