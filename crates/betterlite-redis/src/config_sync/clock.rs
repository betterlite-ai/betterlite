use std::future::Future;
use std::time::Duration;

use tokio::time::{Instant, sleep};

pub trait ConfigSyncClock: Send + Sync {
    fn now(&self) -> Duration;
    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

pub struct MonotonicClock {
    pub origin: Instant,
}

impl ConfigSyncClock for MonotonicClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    async fn pause(&self, duration: Duration) {
        sleep(duration).await;
    }
}
