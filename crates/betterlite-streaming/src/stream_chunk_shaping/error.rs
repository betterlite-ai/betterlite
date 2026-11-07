use thiserror::Error;

// The wrapper raises litellm.InternalServerError for a repeating model, which the proxy answers with a 500.
#[derive(Debug, Error)]
pub enum StreamShapeError {
    #[error("The model is repeating the same chunk = {last_content}.")]
    Repetition {
        last_content: String,
        model: String,
        provider: String,
    },
    #[error("the stream chunk arguments are invalid: {source}")]
    Arguments {
        #[from]
        source: serde_json::Error,
    },
}
