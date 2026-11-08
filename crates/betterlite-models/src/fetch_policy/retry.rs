use std::future::Future;
use std::ops::Range;
use std::str::FromStr as _;

use rust_decimal::Decimal;

use crate::fetch_policy::client::{CatalogClient, CatalogRequest};
use crate::fetch_policy::outcome::{
    AttemptOutcome, FetchResult, Retryable, Unavailable, attempt_fetch_sync, classify_fetch_error,
    classify_fetch_response,
};
use crate::fetch_policy::response::FetchResponse;

pub const MAX_FETCH_ATTEMPTS: u32 = 3;
pub const MAX_FETCH_WAIT_SECONDS: Decimal = Decimal::from_parts(30, 0, 0, false, 0);

pub trait JitterSource: Send {
    fn unit_interval(&mut self) -> Decimal;
}

pub trait CatalogSleeper: Send + Sync {
    fn sleep(&self, seconds: Decimal) -> impl Future<Output = ()> + Send;
}

pub enum NextStep {
    Retry(Decimal),
    Done(FetchResult),
}

#[must_use]
pub fn parse_retry_after_seconds(response: &FetchResponse) -> Option<Decimal> {
    let raw = response.header("retry-after")?;
    let header = raw.trim();
    Decimal::from_str(header)
        .or_else(|_| Decimal::from_scientific(header))
        .map_or_else(
            |_| oversized_retry_after(header),
            |seconds| (seconds >= Decimal::ZERO).then_some(seconds),
        )
}

fn oversized_retry_after(header: &str) -> Option<Decimal> {
    let value = header.parse::<f64>().ok()?;
    if value.is_nan() || value.is_sign_negative() {
        return None;
    }
    if value.is_finite() {
        return Some(Decimal::try_from(value).unwrap_or(MAX_FETCH_WAIT_SECONDS));
    }
    Some(MAX_FETCH_WAIT_SECONDS)
}

#[must_use]
pub fn retry_wait_seconds<J>(retryable: &Retryable, attempt: u32, jitter: &mut J) -> Decimal
where
    J: JitterSource,
{
    if let Some(seconds) = retryable.retry_after_seconds {
        return seconds.min(MAX_FETCH_WAIT_SECONDS);
    }
    let jitter_value = jitter.unit_interval();
    let base = 2_u64.checked_pow(attempt).map(Decimal::from);
    base.and_then(|exponential| exponential.checked_add(jitter_value))
        .map_or(MAX_FETCH_WAIT_SECONDS, |wait| {
            wait.min(MAX_FETCH_WAIT_SECONDS)
        })
}

#[must_use]
pub fn next_retry_step<J>(
    outcome: AttemptOutcome,
    attempt: u32,
    max_attempts: u32,
    jitter: &mut J,
) -> NextStep
where
    J: JitterSource,
{
    let retryable = match outcome {
        AttemptOutcome::Retryable(retryable) => retryable,
        AttemptOutcome::Reloaded(fetched) => return NextStep::Done(FetchResult::Reloaded(fetched)),
        AttemptOutcome::Unavailable(unavailable) => {
            return NextStep::Done(FetchResult::Unavailable(unavailable));
        }
    };
    if attempt == max_attempts {
        return NextStep::Done(FetchResult::Unavailable(Unavailable {
            reason: format!("{} (after {max_attempts} attempts)", retryable.reason),
        }));
    }
    NextStep::Retry(retry_wait_seconds(&retryable, attempt, jitter))
}

pub async fn fetch_with_retry<C, S, J>(
    request: &CatalogRequest,
    max_attempts: u32,
    client: &C,
    sleeper: &S,
    jitter: &mut J,
) -> FetchResult
where
    C: CatalogClient,
    S: CatalogSleeper,
    J: JitterSource,
{
    for attempt in 1..=max_attempts {
        let outcome = match client.get_async(request).await {
            Ok(response) => classify_fetch_response(response, &request.url),
            Err(error) => classify_fetch_error(&error, &request.url),
        };
        match next_retry_step(outcome, attempt, max_attempts, jitter) {
            NextStep::Done(result) => return result,
            NextStep::Retry(seconds) => sleeper.sleep(seconds).await,
        }
    }
    FetchResult::Unavailable(Unavailable {
        reason: "model cost map fetch failed".to_owned(),
    })
}

pub fn fetch_with_retry_sync<C, S, J>(
    request: &CatalogRequest,
    attempts: Range<u32>,
    client: &C,
    sleep: &mut S,
    jitter: &mut J,
) -> FetchResult
where
    C: CatalogClient,
    S: FnMut(Decimal),
    J: JitterSource,
{
    let max_attempts = attempts.end.saturating_sub(1);
    let mut attempt = attempts.start;
    while attempt < attempts.end {
        let outcome = attempt_fetch_sync(client, request);
        match next_retry_step(outcome, attempt, max_attempts, jitter) {
            NextStep::Done(result) => return result,
            NextStep::Retry(seconds) => sleep(seconds),
        }
        attempt = attempt.saturating_add(1);
    }
    FetchResult::Unavailable(Unavailable {
        reason: "model cost map fetch failed".to_owned(),
    })
}
