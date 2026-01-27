use crate::connection::Connection;
use crate::engine::EngineHandler;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Close,
    Shutdown,
}

pub trait EventHandler: Send + Sync + 'static {
    fn init(engine: Arc<EngineHandler>) -> (Self, Action)
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
}
