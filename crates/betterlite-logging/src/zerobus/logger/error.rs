use thiserror::Error;

use crate::logging_failure::FailureError;
use crate::zerobus::row::error::TraceRowError;

#[derive(Debug, Error)]
pub(super) enum EnqueueError {
    #[error("the standard logging payload could not be read: {0}")]
    Details(#[from] FailureError),
    #[error("the standard logging row could not be built: {0}")]
    Row(#[from] TraceRowError),
}
