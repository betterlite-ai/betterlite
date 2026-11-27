use std::error::Error;

use super::error::BatchInputAccountingError;

// The three levels LiteLLM logs an input-file failure at: a 403 is a rejection, every other failure is I/O
pub(super) fn log_failure<CallsError, FilesError, AccessError>(
    file_id: &str,
    error: &BatchInputAccountingError<CallsError, FilesError, AccessError>,
) where
    CallsError: Error + 'static,
    FilesError: Error + 'static,
    AccessError: Error + 'static,
{
    match error.http() {
        Some(http) if http.status == 403 => {
            tracing::warn!(
                "Batch rejected: caller not authorized for a model named in {}: {}",
                file_id,
                serde_json::to_string(&http.detail).unwrap_or_default()
            );
        }
        Some(http) => {
            tracing::error!(
                "Batch input file rejected for {}: status={} detail={}",
                file_id,
                http.status,
                serde_json::to_string(&http.detail).unwrap_or_default()
            );
        }
        None => {
            tracing::error!("Error counting input file usage for {}: {}", file_id, error);
        }
    }
}
