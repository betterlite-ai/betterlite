mod attempt;
mod budget;
pub mod lookup;
pub mod resyncs;
mod tests;

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

pub const READ_THROUGH_MISS_TTL: Duration = Duration::from_secs(2);
pub const READ_THROUGH_RESYNC_WINDOW: Duration = Duration::from_secs(5);
pub const READ_THROUGH_MAX_RESYNCS: usize = 20;
pub const MAX_RECENT_MISSES: usize = 1000;

pub trait Resync: Send + Sync {
    fn resync(&self, key: &str) -> impl Future<Output = Result<bool, ResyncError>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum ResyncError {
    #[error("the registry resync failed: {message}")]
    Failed { message: String },
}

pub struct RegistryReadThrough<R: Resync> {
    resync: R,
    miss_ttl: Duration,
    max_resyncs_per_window: usize,
    window: Duration,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    recent_misses: RecentMisses,
    window_started_at: Option<Instant>,
    window_resyncs: usize,
}

#[derive(Debug, Default)]
struct RecentMisses {
    entries: BTreeMap<String, Instant>,
}
