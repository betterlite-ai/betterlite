use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

// Wall-clock seconds since the epoch, because a stale copy is measured against the same clock Redis stored it with.
#[async_trait]
pub trait Clock: Send + Sync {
    fn now(&self) -> f64;
    async fn sleep(&self, delay: Duration);
}

pub struct SystemClock;

#[async_trait]
impl Clock for SystemClock {
    fn now(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0_f64, |elapsed| elapsed.as_secs_f64())
    }

    async fn sleep(&self, delay: Duration) {
        tokio::time::sleep(delay).await;
    }
}
