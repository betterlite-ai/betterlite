use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::router_core::headers::{Prepared, ensure_additional_headers};
use crate::routing_retries::trail::RetryTrail;

const ATTEMPTED_RETRIES: &str = "x-litellm-attempted-retries";
const MAX_RETRIES: &str = "x-litellm-max-retries";

// add_retry_headers_to_response: a first-attempt success names no maximum, a retried one names both counts
pub fn install_retry_headers(prepared: &mut Prepared, trail: &RetryTrail) {
    let headers = ensure_additional_headers(prepared);
    headers.insert(
        ATTEMPTED_RETRIES.to_owned(),
        Json::Number(Number::from(trail.attempted_retries)),
    );
    if trail.attempted_retries > 0 {
        headers.insert(
            MAX_RETRIES.to_owned(),
            Json::Number(Number::from(trail.max_retries)),
        );
    }
}
