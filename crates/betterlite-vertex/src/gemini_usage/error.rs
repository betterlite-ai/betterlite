#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("usageMetadata is required in the response")]
    MissingUsageMetadata,
    #[error("{field} overflows the token count range")]
    Overflow { field: &'static str },
}
