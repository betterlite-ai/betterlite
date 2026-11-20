use betterlite_logging::spend_sanitize::REDACTED_BY_LITELM as LOGGING_MARKER;

pub mod error;
pub mod kill_switch;
pub mod params;
pub mod redact;
pub mod response;
pub mod restore;

#[path = "agent_credentials/tests/support.rs"]
mod support;

pub const REDACTED_BY_LITELM: &str = LOGGING_MARKER;
