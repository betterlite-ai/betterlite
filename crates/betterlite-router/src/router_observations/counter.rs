use std::time::Duration;

use thiserror::Error;

// one item of LiteLLM's async_increment_cache_pipeline: a key, the delta, and the window's ttl
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Increment {
    pub key: String,
    pub value: i64,
    pub ttl: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CounterError {
    #[error("the counter store rejected the write to {key}")]
    Rejected { key: String },
}

pub trait CounterSink: Send + Sync {
    // cache.increment_cache(key, value, ttl), the call the per-minute success, failure and rpm counters make
    fn increment(&self, key: &str, value: i64, ttl: Duration) -> Result<(), CounterError>;
    // cache.async_increment_cache_pipeline: every item lands with its own ttl, or the whole call fails
    fn increment_pipeline(&self, increments: &[Increment]) -> Result<(), CounterError>;
    // cache.get_cache(key, local_only=True)
    fn read(&self, key: &str) -> Option<i64>;
    // cache.set_cache(key, value, local_only=True), which keeps the stored ttl when none is passed
    fn write(&self, key: &str, value: i64, ttl: Option<Duration>);
}
