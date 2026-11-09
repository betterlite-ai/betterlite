use std::time::Duration;

use async_trait::async_trait;
use tokio::time::sleep;

#[async_trait]
pub trait UploadSleeper: Send + Sync {
    async fn sleep(&self, delay: Duration);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokioSleeper;

#[async_trait]
impl UploadSleeper for TokioSleeper {
    async fn sleep(&self, delay: Duration) {
        sleep(delay).await;
    }
}
