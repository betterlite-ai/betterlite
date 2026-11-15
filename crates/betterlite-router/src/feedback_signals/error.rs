use thiserror::Error;

#[derive(Debug, Error)]
pub enum SignalError {
    #[error("the pinned feedback signal patterns do not compile: {source}")]
    Pattern {
        #[source]
        source: regex::Error,
    },
    #[error("the {field} detector threshold must be a finite number, got {value}")]
    NonFinite { field: &'static str, value: f64 },
}
