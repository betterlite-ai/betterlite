use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;

use super::logger::DatadogLogger;

#[must_use]
pub const fn get_request_response_payload(
    _logger: &DatadogLogger,
    _request_id: &str,
    _start_time_utc: Option<DateTime<Utc>>,
    _end_time_utc: Option<DateTime<Utc>>,
) -> Option<IndexMap<String, Json>> {
    // LiteLLM's AdditionalLoggingUtils dispatch reads None as this logger keeping no request or response payloads
    None
}
