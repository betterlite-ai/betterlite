use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use serde::Serialize;

use super::error::WatsonxEmbeddingError;

// IBMWatsonXEmbeddingConfig.get_supported_openai_params returns nothing, so every option reaches the request as given
pub const SUPPORTED_OPENAI_PARAMS: [&str; 0] = [];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WatsonxEmbeddingBody {
    pub inputs: Vec<String>,
    pub parameters: WatsonxParams,
    #[serde(flatten)]
    pub identity: WatsonxParams,
}

pub fn take_api_version(
    options: &mut WatsonxParams,
) -> Result<Option<String>, WatsonxEmbeddingError> {
    match options.remove("api_version") {
        None => Ok(None),
        Some(value) => serde_json::from_value::<Option<String>>(value).map_err(|source| {
            WatsonxEmbeddingError::InvalidField {
                field: "api_version",
                source,
            }
        }),
    }
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
