use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use serde::Serialize;

use super::error::WatsonxEmbeddingError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WatsonxEmbeddingBody {
    pub inputs: Vec<String>,
    pub parameters: WatsonxParams,
    #[serde(flatten)]
    pub identity: WatsonxParams,
}

pub fn inputs(input: &EmbeddingInput) -> Result<Vec<String>, WatsonxEmbeddingError> {
    match input {
        EmbeddingInput::Text(text) => Ok(vec![text.clone()]),
        EmbeddingInput::Texts(texts) => Ok(texts.clone()),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            Err(WatsonxEmbeddingError::UnsupportedInput)
        }
    }
}
