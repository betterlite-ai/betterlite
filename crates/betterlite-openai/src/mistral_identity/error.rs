use thiserror::Error;

use crate::openai_base_config::Headers;

#[derive(Debug, Error)]
pub enum MistralIdentityError {
    #[error("no Mistral API key is set; pass api_key or set MISTRAL_API_KEY")]
    MissingApiKey,
    #[error("the Mistral API returned {status}: {message}")]
    Api {
        status: u16,
        message: String,
        headers: Headers,
    },
}
