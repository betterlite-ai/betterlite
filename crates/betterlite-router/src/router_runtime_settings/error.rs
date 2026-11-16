use thiserror::Error;

#[derive(Debug, Error)]
pub enum RouterSettingsError {
    #[error("router setting {key} is not an integer")]
    NotAnInteger { key: String },
    #[error("router setting retry_policy field {field} is not a valid retry count: {source}")]
    RetryPolicy {
        field: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{name} failed: {message}")]
    Target { name: String, message: String },
}
