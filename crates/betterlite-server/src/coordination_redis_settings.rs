pub mod catalog;
pub mod decode;
pub mod projection;
pub mod source;
pub mod store;

mod tests;
pub mod validate;

pub const REDACTED_VALUE: &str = "***REDACTED***";

// The three fields litellm treats as credentials, url included because a Redis url embeds a password.
pub const SENSITIVE_FIELDS: [&str; 3] = ["password", "sentinel_password", "url"];
