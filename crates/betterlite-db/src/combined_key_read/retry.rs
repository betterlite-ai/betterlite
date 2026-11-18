use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::database_failures::deadline::{DbLookupDeadlineExceeded, DbLookupError};
use crate::database_faults::{DbError, ErrorType};
use crate::key_user_writes::retry::MAX_TIME;
use crate::migration_ledger::lock::JitterSource;

use super::error::{KeyReadError, READ_CALL_TYPE};

// backoff.expo with full jitter: 2^(tries-1) seconds scaled by uniform(0,1), capped by the time left
pub(crate) fn retry_wait(jitter: &dyn JitterSource, tries: u32, elapsed: Duration) -> Duration {
    let exponent = tries.saturating_sub(1).min(31);
    let seconds = 1_u32.checked_shl(exponent).unwrap_or(u32::MAX);
    let value = Duration::from_secs(u64::from(seconds));
    let nanos = u64::try_from(value.as_nanos()).unwrap_or(u64::MAX);
    let jittered = Duration::from_nanos(jitter.uniform_nanos(nanos));
    jittered.min(MAX_TIME.saturating_sub(elapsed))
}

// log_db_metrics takes a lookup error, so a failure that is not a database fault travels in a parked slot
pub(crate) fn parked_lookup_error() -> DbLookupError {
    DbLookupError::Deadline(DbLookupDeadlineExceeded {
        lookup: String::from(READ_CALL_TYPE),
        deadline_seconds: 0.0,
        cause: None,
    })
}

pub(crate) fn take_parked(parked: Mutex<Option<KeyReadError>>) -> KeyReadError {
    parked
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .unwrap_or_else(|| {
            KeyReadError::Database(DbError {
                error_type: ErrorType::Other,
                message: String::from("the database lookup lost its original error"),
                code: None,
                meta_code: None,
                traceback_modules: Vec::new(),
                cause: None,
            })
        })
}
