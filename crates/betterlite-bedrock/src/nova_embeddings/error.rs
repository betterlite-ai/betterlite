use thiserror::Error;

#[derive(Debug, Error)]
pub enum NovaEmbeddingsError {
    #[error("Invalid data URL format: {prefix}...")]
    InvalidDataUrl { prefix: String },
    #[error("Invalid data URL format (missing comma): {prefix}...")]
    MissingComma { prefix: String },
    #[error("output_s3_uri is required for async invoke requests")]
    OutputS3UriRequired,
    #[error("malformed Nova response field: {field}")]
    MalformedField { field: &'static str },
    #[error("token estimate overflow")]
    TokenOverflow,
}
