use std::future::Future;
use std::time::Duration;

use tokio::time::sleep;

pub trait BackoffClock: Send + Sync {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

pub struct TokioClock;

impl BackoffClock for TokioClock {
    async fn sleep(&self, duration: Duration) {
        sleep(duration).await;
    }
}
