use std::error::Error as StdError;
use std::time::{Duration, Instant};

use deadpool_postgres::PoolError;
use tokio_postgres::error::SqlState;

use crate::database_faults::native::{
    NativeFault, classify_sqlstate, is_native_transport_error, native_sqlstate,
};
use crate::key_user_writes::retry::WriteClock;
use crate::migration_ledger::lock::JitterSource;
use crate::postgres_store::errors::DbError;
use crate::spend_log_storage::rows::writer::{
    NON_BLOCKING_PREFIX, SpendLogFailureEvent, SpendLogFailureSink, UPDATE_SPEND_CALL_TYPE,
};
use crate::tool_spend::RetryPause;

const JITTER_NANOS: u64 = 1_000_000_000;

// What _spend_commit_failure_is_requeue_safe and the retry rule decide about one failed chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendFailure {
    NotSent,
    Ambiguous,
    Deadlock,
    Rejected,
    Other,
}

impl SpendFailure {
    // _spend_commit_failure_is_requeue_safe: only a proved-unapplied or refused statement can be sent again
    #[must_use]
    pub const fn is_requeue_safe(self) -> bool {
        match self {
            Self::NotSent | Self::Deadlock | Self::Other => true,
            Self::Ambiguous | Self::Rejected => false,
        }
    }

    // _handle_spend_update_failure's is_retryable: only a proved-unapplied connection error or a deadlock earns another attempt
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        match self {
            Self::NotSent | Self::Deadlock => true,
            Self::Ambiguous | Self::Rejected | Self::Other => false,
        }
    }
}

pub trait SpendUpdateFailure: StdError + Send + Sync + 'static {
    fn spend_failure(&self) -> SpendFailure;
}

pub(crate) fn failure_for_sqlstate(code: &SqlState) -> SpendFailure {
    if code.code().starts_with("22") || code.code().starts_with("23") {
        return SpendFailure::Rejected;
    }
    if classify_sqlstate(code) == NativeFault::Deadlock {
        return SpendFailure::Deadlock;
    }
    SpendFailure::Other
}

#[must_use]
pub fn classify_spend_failure<E>(error: &DbError<E>) -> SpendFailure {
    if is_native_transport_error(error) {
        return if proven_not_sent(error) {
            SpendFailure::NotSent
        } else {
            SpendFailure::Ambiguous
        };
    }
    native_sqlstate(error).map_or(SpendFailure::Other, |code| failure_for_sqlstate(&code))
}

fn proven_not_sent<E>(error: &DbError<E>) -> bool {
    match error {
        DbError::NoDbConnected => true,
        DbError::Pool { source } => match &source.driver {
            PoolError::Timeout(_) | PoolError::Closed | PoolError::Backend(_) => true,
            PoolError::NoRuntimeSpecified | PoolError::PostCreateHook(_) => false,
        },
        DbError::SqlState { code, .. } => {
            classify_sqlstate(code) == NativeFault::ConnectionUnavailable
        }
        DbError::UniqueViolation { .. }
        | DbError::Backend { .. }
        | DbError::InvalidRecord { .. }
        | DbError::CreateNull => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendRetry {
    Retry,
    Stop,
}

#[derive(Clone, Copy)]
pub struct SpendRetryPolicy<'run> {
    pub retries: u32,
    pub clock: &'run dyn WriteClock,
    pub jitter: &'run dyn JitterSource,
    pub pause: &'run dyn RetryPause,
    pub failures: &'run dyn SpendLogFailureSink,
}

// _handle_spend_update_failure: one more attempt for a proved-unapplied connection error or a deadlock, else report and hand the error back.
#[must_use]
pub async fn handle_spend_update_failure<Failure>(
    policy: &SpendRetryPolicy<'_>,
    error: &Failure,
    attempt: u32,
    started: Instant,
) -> SpendRetry
where
    Failure: SpendUpdateFailure,
{
    let failure = error.spend_failure();
    if !failure.is_retryable() || attempt >= policy.retries {
        let elapsed = policy.clock.instant().saturating_duration_since(started);
        report_failed_update_spend(policy.failures, error, elapsed);
        return SpendRetry::Stop;
    }
    tracing::warn!(
        attempt = attempt.saturating_add(1),
        retries = policy.retries,
        error = %error,
        "Retrying spend update after retryable DB error"
    );
    policy
        .pause
        .pause(retry_delay(policy.jitter, attempt))
        .await;
    SpendRetry::Retry
}

// _raise_failed_update_spend_exception: report the non-blocking failure through the sink and keep the whole cause chain.
pub fn report_failed_update_spend(
    failures: &dyn SpendLogFailureSink,
    error: &dyn StdError,
    duration: Duration,
) {
    let mut messages = vec![error.to_string()];
    let mut cause = error.source();
    while let Some(current) = cause {
        messages.push(current.to_string());
        cause = current.source();
    }
    let chain = messages.join("\nCaused by: ");
    let diagnostic = format!("{NON_BLOCKING_PREFIX} {chain}");
    tracing::error!("{diagnostic}");
    failures.emit(SpendLogFailureEvent {
        call_type: UPDATE_SPEND_CALL_TYPE,
        duration,
        diagnostic,
    });
}

pub(crate) fn retry_delay(jitter: &dyn JitterSource, attempt: u32) -> Duration {
    let base = 1_u64.checked_shl(attempt).unwrap_or(u64::MAX);
    let span = base.saturating_mul(JITTER_NANOS);
    Duration::from_secs(base).saturating_add(Duration::from_nanos(jitter.uniform_nanos(span)))
}

mod tests;
