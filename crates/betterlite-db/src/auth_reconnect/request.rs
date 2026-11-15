use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use secrecy::SecretString;

#[derive(Debug, thiserror::Error)]
pub enum ReconnectError<EngineError>
where
    EngineError: Error + Send + Sync + 'static,
{
    #[error("DATABASE_URL not set")]
    MissingDatabaseUrl,
    #[error("the database reconnect timed out")]
    Timeout,
    #[error("the database replacement was declined by the generation guard")]
    ForcedDecline,
    #[error(transparent)]
    Engine(EngineError),
}

pub struct ReconnectRequest<'request> {
    pub reason: &'request str,
    pub force: bool,
    pub force_recreate: bool,
    pub timeout: Option<Duration>,
    pub lock_timeout: Option<Duration>,
    pub database_url: Option<&'request SecretString>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectSnapshot {
    pub consecutive_failures: u32,
    pub engine_dead: bool,
}

pub struct ReconnectHistory<Handle> {
    pub last_attempt: Option<Instant>,
    pub consecutive_failures: u32,
    pub engine_dead: bool,
    pub failed_repairs: Vec<(Arc<Handle>, u64)>,
}

impl<Handle> Default for ReconnectHistory<Handle> {
    fn default() -> Self {
        Self {
            last_attempt: None,
            consecutive_failures: 0,
            engine_dead: false,
            failed_repairs: Vec::new(),
        }
    }
}
