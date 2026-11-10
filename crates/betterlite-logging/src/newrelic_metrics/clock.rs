use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// LiteLLM reads time.time() at every flush and drain, so the clock is a closure the caller injects
pub type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

#[must_use]
pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
    })
}
