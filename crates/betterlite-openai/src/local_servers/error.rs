use thiserror::Error;

use crate::chat_content::ChatContentError;

#[derive(Debug, Error)]
pub enum LocalServerError {
    #[error("a chat message could not be converted: {source}")]
    MessageShape {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Content(#[from] ChatContentError),
    #[error(
        "Parameter {param} is not supported for model {model}. Supported parameters are {supported:?}. Set drop_params=True to drop unsupported parameters."
    )]
    UnsupportedParam {
        param: String,
        model: String,
        supported: Vec<&'static str>,
    },
    #[error("the embedding input could not be written as JSON: {source}")]
    Input {
        #[source]
        source: serde_json::Error,
    },
}
