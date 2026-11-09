use thiserror::Error;

#[derive(Debug, Error)]
pub enum AzureError {
    #[error("messages has more than {limit} entries")]
    TooManyMessages { limit: usize },
    #[error("a message has more than {limit} content parts")]
    TooManyParts { limit: usize },
    #[error("message {index} function_call must be an object")]
    FunctionCall { index: usize },
    #[error("message {index} function_call.arguments must be a string")]
    Arguments { index: usize },
    #[error("message {index} content part {part} has no image_url")]
    MissingImageUrl { index: usize, part: usize },
    #[error("message {index} content part {part} image_url must be a string or an object")]
    ImageUrl { index: usize, part: usize },
    #[error("message {index} content part {part} file must be an object")]
    File { index: usize, part: usize },
    #[error("message {index} content part {part} text must be a string")]
    Text { index: usize, part: usize },
    #[error("message {index} is a tool message without content")]
    MissingToolContent { index: usize },
    #[error("the wrapped tool result cannot be serialized")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}
