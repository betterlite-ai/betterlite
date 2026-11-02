#![expect(
    clippy::disallowed_types,
    reason = "stored cache_settings is a free-form provider kwargs dict with no fixed schema"
)]

pub mod merge;
pub mod parse;
pub mod precedence;
pub mod redact;
pub mod reuse;
pub mod target;
mod tests;

pub(crate) const REDACTED_VALUE: &str = "***REDACTED***";

pub(crate) const CACHE_SENSITIVE_FIELDS: [&str; 3] = ["password", "sentinel_password", "url"];

pub(crate) const URL_OVERRIDDEN_FIELDS: [&str; 5] = ["host", "port", "db", "password", "username"];

pub(crate) const CONNECTION_TARGET_FIELDS: [&str; 6] = [
    "host",
    "port",
    "url",
    "redis_startup_nodes",
    "sentinel_nodes",
    "service_name",
];
