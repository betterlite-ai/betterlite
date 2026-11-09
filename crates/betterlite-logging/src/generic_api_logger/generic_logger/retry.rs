use super::GenericApiLogger;
use super::error::ApiError;
use super::transport::PostRequest;
use async_trait::async_trait;
use std::ops::Mul as _;
use std::time::Duration;
use tokio::time::sleep;
mod tests;
#[async_trait]
pub trait RetrySleeper: Send + Sync {
    async fn sleep(&self, duration: Duration);
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemSleeper;
#[async_trait]
impl RetrySleeper for SystemSleeper {
    async fn sleep(&self, duration: Duration) {
        sleep(duration).await;
    }
}

async fn post_once(logger: &GenericApiLogger, body: &str) -> Result<(), ApiError> {
    let request = PostRequest {
        endpoint: logger.settings.endpoint.clone(),
        headers: logger.settings.headers.clone(),
        body: body.to_owned(),
        timeout: logger.settings.timeout,
    };
    let status = logger.transport.post(request).await?;
    if (200..300).contains(&status) {
        return Ok(());
    }
    Err(ApiError::Status { status })
}

async fn sleep_before_retry(seconds: f64, attempt: u32, clock: &dyn RetrySleeper) {
    if seconds <= 0.0_f64 {
        return;
    }
    let exponent = i32::try_from(attempt).unwrap_or(i32::MAX);
    let Ok(delay) = Duration::try_from_secs_f64(seconds.mul(2_f64.powi(exponent))) else {
        tracing::warn!(
            "Generic API Logger - the retry delay for attempt {attempt} is not a usable duration"
        );
        return;
    };
    clock.sleep(delay).await;
}

pub(super) async fn post_with_retries(
    logger: &GenericApiLogger,
    body: &str,
) -> Result<(), ApiError> {
    let total = logger.settings.max_retries.saturating_add(1);
    let mut attempt: u32 = 0;
    loop {
        let Err(error) = post_once(logger, body).await else {
            return Ok(());
        };
        if attempt >= logger.settings.max_retries || !error.is_retryable() {
            return Err(error);
        }
        tracing::warn!(
            "Generic API Logger - retrying request to {} after error: {} (attempt {}/{})",
            logger.settings.endpoint,
            error,
            attempt.saturating_add(1),
            total
        );
        sleep_before_retry(
            logger.settings.retry_delay,
            attempt,
            logger.sleeper.as_ref(),
        )
        .await;
        attempt = attempt.saturating_add(1);
    }
}
