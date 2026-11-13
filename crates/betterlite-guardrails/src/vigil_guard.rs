use std::time::Duration;

use reqwest::StatusCode;

use self::analysis::Decision;
use self::handler_protocol::RequestTimeout;

pub mod analysis;
pub mod block_reason;
pub mod config;
pub mod error;
pub mod failure;
pub mod handler_protocol;
pub mod metadata;
pub mod output;
pub mod retry;
pub mod sanitize;
pub mod tool_calls;
pub mod transient;

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
