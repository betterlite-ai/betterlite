use thiserror::Error;

#[derive(Debug, Error)]
pub enum XaiIdentityError {
    #[error("XAI API base or key is not set. Set XAI_API_BASE and provide an xAI API key.")]
    MissingCredentials,
    #[error("Failed to fetch models from XAI. Status code: {status}, Response: {body}")]
    Status { status: u16, body: String },
    #[error("xAI model list is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("xAI model list is not a valid catalog: {0}")]
    Decode(#[from] serde_json::Error),
}
