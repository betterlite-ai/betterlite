use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin settings entry {index} is invalid: {source}")]
    InvalidEntry {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("Invalid, tampered, or expired plugin session claim")]
    InvalidClaim,
    #[error("Plugin claim audience mismatch")]
    AudienceMismatch,
    #[error("Plugin session claim expired")]
    ClaimExpired,
    #[error("the plugin claim key could not be derived: {source}")]
    ClaimKey {
        #[source]
        source: hmac::digest::InvalidLength,
    },
    #[error("the forwarded header {name} has an invalid value")]
    HeaderValue { name: &'static str },
}
