use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::time::sleep;

use super::IndexBuilder;
use super::database::{DatabaseFailure as _, DatabaseFuture, IndexDatabase};

pub(crate) const LOCK_ATTEMPTS: u32 = 10;
pub(crate) const RETRY_HALF_BASE_MILLIS: u64 = 125;
pub(crate) const RETRY_HALF_MAX_MILLIS: u64 = 4_000;
pub(crate) const HANDOVER: Duration = Duration::from_secs(2);
const APPLY_LOCK_TIMEOUT: &str = "SET lock_timeout = '200ms'";
const RESET_LOCK_TIMEOUT: &str = "SET lock_timeout = 0";

pub(crate) trait RetryPause: Send + Sync {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TokioPause;

impl RetryPause for TokioPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(sleep(delay))
    }
}

// Mirrors _with_bounded_lock: a short lock_timeout per attempt, the coordinator held per attempt only
pub(crate) async fn with_bounded_lock<'step, D, S, F>(
    builder: &'step IndexBuilder<'_, D>,
    what: &str,
    step: S,
) -> Result<bool, D::Error>
where
    D: IndexDatabase,
    S: Fn() -> F,
    F: Future<Output = Result<bool, D::Error>> + Send + 'step,
{
    for attempt in 0..LOCK_ATTEMPTS {
        if attempt > 0 {
            builder
                .dependencies
                .pause
                .pause(retry_pause(builder.dependencies, attempt))
                .await;
        }
        builder.database.execute(APPLY_LOCK_TIMEOUT).await?;
        let attempt_step: DatabaseFuture<'step, bool, D::Error> = Box::pin(step());
        let outcome = builder.database.coordinate(attempt_step).await;
        builder.database.execute(RESET_LOCK_TIMEOUT).await?;
        match outcome {
            Ok(result) => return Ok(result),
            Err(error) if error.is_lock_not_available() => {
                tracing::info!("Waiting for open transactions before {what}");
            }
            Err(error) => return Err(error),
        }
    }
    tracing::warn!(
        "Could not get the lock for {what} without holding writes back, leaving it for the next index build"
    );
    Ok(false)
}

// The pause ranges over min(8s, 0.25s * 2**attempt) * uniform(0.5, 1.0) seconds
fn retry_pause(dependencies: &super::IndexDependencies<'_>, attempt: u32) -> Duration {
    let half = retry_half(attempt);
    let span = u64::try_from(half.as_nanos()).unwrap_or(u64::MAX);
    half.saturating_add(Duration::from_nanos(
        dependencies.jitter.uniform_nanos(span),
    ))
}

fn retry_half(attempt: u32) -> Duration {
    let millis = RETRY_HALF_BASE_MILLIS
        .checked_shl(attempt)
        .unwrap_or(RETRY_HALF_MAX_MILLIS)
        .min(RETRY_HALF_MAX_MILLIS);
    Duration::from_millis(millis)
}
