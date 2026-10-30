#[derive(Debug, Clone, thiserror::Error)]
pub enum RedactionError {
    #[error("secret redaction pattern failed to compile: {source}")]
    Pattern {
        #[source]
        source: regex::Error,
    },
}
