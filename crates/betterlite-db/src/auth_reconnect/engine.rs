use std::error::Error;

use secrecy::SecretString;

use crate::database_refresh::generation::Observation;
use crate::database_refresh::handle::PoolFuture;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    Alive,
    Dead,
    Unknown,
}

pub trait ReconnectEngine: Send + Sync {
    type Handle: Send + Sync + 'static;
    type Error: Error + Send + Sync + 'static;

    fn writer(&self) -> Observation<Self::Handle>;
    fn read_target(&self) -> Observation<Self::Handle>;
    fn engine_state(&self) -> EngineState;
    fn recreate(
        &self,
        url: &SecretString,
        expected_generation: Option<u64>,
    ) -> PoolFuture<'_, bool, Self::Error>;
    fn writer_read_only(&self) -> PoolFuture<'_, bool, Self::Error>;
    fn writer_select_one(&self) -> PoolFuture<'_, (), Self::Error>;
    fn mark_writer_recovered(&self);
    fn reap(&self);
    fn cleanup_watcher(&self);
    fn rearm_watcher(&self) -> PoolFuture<'_, (), Self::Error>;
}
