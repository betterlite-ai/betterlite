use std::time::Duration;

use reqwest::StatusCode;

use self::vigil_guard_analysis::Decision;
use self::vigil_guard_handler_protocol::RequestTimeout;

pub mod vigil_guard_analysis;
pub mod vigil_guard_block_reason;
pub mod vigil_guard_config;
pub mod vigil_guard_error;
pub mod vigil_guard_failure;
pub mod vigil_guard_handler_protocol;
pub mod vigil_guard_metadata;
pub mod vigil_guard_output;
pub mod vigil_guard_retry;
pub mod vigil_guard_sanitize;
pub mod vigil_guard_tool_calls;
pub mod vigil_guard_transient;

pub const ANALYZE_ENDPOINT: &str = "/v1/guard/analyze";
pub const BLOCK_REASON_MAX_CHARS: usize = 500;
pub const METADATA_STRING_MAX_CHARS: usize = 500;
pub const METADATA_ARRAY_MAX_ITEMS: usize = 10;
pub const VALID_DECISIONS: [Decision; 3] =
    [Decision::Allowed, Decision::Sanitized, Decision::Blocked];
pub const TRANSIENT_STATUS_CODES: [StatusCode; 4] = [
    StatusCode::TOO_MANY_REQUESTS,
    StatusCode::BAD_GATEWAY,
    StatusCode::SERVICE_UNAVAILABLE,
    StatusCode::GATEWAY_TIMEOUT,
];
pub const METADATA_ALLOWLIST: [&str; 12] = [
    "model",
    "model_group",
    "provider",
    "region",
    "deployment",
    "user",
    "user_id",
    "session_id",
    "conversation_id",
    "request_id",
    "tenant_id",
    "org_id",
];
pub const DEFAULT_TIMEOUT: RequestTimeout = RequestTimeout {
    total: Duration::from_secs(10),
    connect: Duration::from_secs(5),
};
