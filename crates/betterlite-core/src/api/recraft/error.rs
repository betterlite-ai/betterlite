use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecraftError {
    #[error("RECRAFT_API_KEY is not set")]
    MissingApiKey,
    #[error("parameter {parameter} is not supported for model {model}")]
    UnsupportedParameter { parameter: String, model: String },
    #[error("{argument} is given both as an argument and as a provider option")]
    DuplicateArgument { argument: &'static str },
}
