use std::future::Future;
use std::time::Duration;

use tokio::time::timeout;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("Health check timeout exceeded")]
pub struct HealthCheckTimeout;

pub async fn run_with_timeout<F, T>(task: F, duration: Duration) -> Result<T, HealthCheckTimeout>
where
    F: Future<Output = T>,
{
    timeout(duration, task)
        .await
        .map_err(|_elapsed| HealthCheckTimeout)
}
