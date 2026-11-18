pub mod error;
pub mod event;
pub mod service;
pub mod spend_redis;

mod tests;

use std::time::Duration;

// budget_limiter.DEFAULT_REDIS_SYNC_INTERVAL: the limiter's own one-second constant, not a router setting
pub const DEFAULT_REDIS_SYNC_INTERVAL: Duration = Duration::from_secs(1);

// async_log_success_event skips the two websocket session wrappers; the turns inside them already spend
pub const SKIPPED_CALL_TYPES: [&str; 2] = ["_aresponses_websocket", "_arealtime"];
