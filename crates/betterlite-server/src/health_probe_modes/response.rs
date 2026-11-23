use serde::Serialize;
use std::collections::BTreeMap;
pub const REMAINING_REQUESTS_HEADER: &str = "x-ratelimit-remaining-requests";
pub const REMAINING_TOKENS_HEADER: &str = "x-ratelimit-remaining-tokens";
pub const REGION_HEADER: &str = "x-ms-region";
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct HealthCheckResponse {
    #[serde(
        rename = "x-ratelimit-remaining-requests",
        skip_serializing_if = "Option::is_none"
    )]
    pub remaining_requests: Option<String>,
    #[serde(
        rename = "x-ratelimit-remaining-tokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub remaining_tokens: Option<String>,
    #[serde(rename = "x-ms-region", skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}
#[must_use]
pub fn health_check_response(headers: &BTreeMap<String, String>) -> HealthCheckResponse {
    HealthCheckResponse {
        remaining_requests: headers.get(REMAINING_REQUESTS_HEADER).cloned(),
        remaining_tokens: headers.get(REMAINING_TOKENS_HEADER).cloned(),
        region: headers.get(REGION_HEADER).cloned(),
    }
}
