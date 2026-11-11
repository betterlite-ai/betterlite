use std::io;

#[derive(Debug, thiserror::Error)]
pub enum TraceLifecycleError {
    #[error("missing setting {setting}")]
    MissingSetting { setting: String },
    #[error("agent tracing storage failed: {source}")]
    Storage {
        #[source]
        source: io::Error,
    },
    #[error("agent tracing receiver failed: {message}")]
    Runtime { message: String },
    #[error("invalid agent tracing value: {message}")]
    InvalidValue { message: String },
    #[error("agent tracing failed: {message}")]
    Unexpected { message: String },
}
