use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::migration_ledger::lock::JitterSource;

use super::KeyUserWriteError;

pub const MAX_TRIES: u32 = 3;
pub const MAX_TIME: Duration = Duration::from_secs(10);
pub const FAILURE_REPORT_DEADLINE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallType {
    InsertData,
    UpdateData,
}

impl CallType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InsertData => "insert_data",
            Self::UpdateData => "update_data",
        }
    }

    const fn heading(self) -> &'static str {
        match self {
            Self::InsertData => "LiteLLM Prisma Client Exception in insert_data",
            Self::UpdateData => "LiteLLM Prisma Client Exception - update_data",
        }
    }
}

pub trait WriteClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;

    fn instant(&self) -> Instant;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemWriteClock;

impl WriteClock for SystemWriteClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn instant(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug)]
pub struct FailureEvent<'error> {
    pub call_type: CallType,
    pub error: &'error KeyUserWriteError,
    pub traceback: String,
    pub duration: Duration,
}

pub type FailureFuture<'sink> = Pin<Box<dyn Future<Output = ()> + Send + 'sink>>;

// the source detaches failure_handler from the failed write, so a sink never borrows the write it reports
pub trait FailureSink: Send + Sync {
    fn record(&self, event: FailureEvent<'_>) -> FailureFuture<'_>;
}

// backoff's expo wait doubled per attempt, jittered across the whole range and capped by the ten second budget
pub(super) fn retry_wait(jitter: &dyn JitterSource, tries: u32, elapsed: Duration) -> Duration {
    let exponent = tries.saturating_sub(1).min(31);
    let seconds = 1_u32.checked_shl(exponent).unwrap_or(u32::MAX);
    let value = Duration::from_secs(u64::from(seconds));
    let nanos = u64::try_from(value.as_nanos()).unwrap_or(u64::MAX);
    let jittered = Duration::from_nanos(jitter.uniform_nanos(nanos));
    jittered.min(MAX_TIME.saturating_sub(elapsed))
}

pub(super) fn traceback(call_type: CallType, error: &dyn Error) -> String {
    let mut rendered = format!("{}: {error}", call_type.heading());
    let mut cause = error.source();
    while let Some(source) = cause {
        rendered.push_str("\nCaused by: ");
        rendered.push_str(&source.to_string());
        cause = source.source();
    }
    rendered
}
