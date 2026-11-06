use super::chain::{Failure, Origin};
use crate::database_faults::{self, ErrorType};

pub const STALE_PLAN_MARKER: &str = "cached plan must not change result type";

#[must_use]
pub fn is_service_unavailable(failure: &Failure) -> bool {
    let stale_plan = failure.message.to_lowercase().contains(STALE_PLAN_MARKER);
    match &failure.origin {
        Origin::Database(error) => {
            database_faults::is_infrastructure_error(error.error_type)
                || database_faults::is_database_transport_error(error)
                || database_faults::is_engine_internal_error(error)
                || stale_plan
                // a lookup deadline is a DBLookupDeadlineExceeded, which subclasses asyncio.TimeoutError
                || error.error_type == ErrorType::Deadline
        }
        Origin::Os | Origin::AsyncTimeout | Origin::Asyncpg(_) => true,
        Origin::AsyncpgUnavailable(_) | Origin::Other | Origin::BaseException => stale_plan,
    }
}

#[must_use]
pub fn is_permanent_fault(failure: &Failure) -> bool {
    if let Origin::Database(error) = &failure.origin {
        return database_faults::is_permanent_database_fault(error);
    }
    false
}
