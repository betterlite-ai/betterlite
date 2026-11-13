use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VertexTextEmbeddingError {
    #[error("the embeddings input must be text, not tokens")]
    InputNotText,
    #[error("the optional parameters must be an object")]
    OptionalParams,
    #[error("the {field} value must be text")]
    NotText { field: &'static str },
    #[error("{value} is not a Vertex embedding task type")]
    TaskType { value: String },
    #[error("the embeddings answer has no predictions")]
    MissingPredictions,
    #[error("the embeddings predictions must be a list")]
    PredictionsNotList,
    #[error("the prediction at index {index} is not a list")]
    PredictionNotList { index: usize },
    #[error("the prediction at index {index} has no vector")]
    MissingVector { index: usize },
    #[error("the embeddings answer has no {field}")]
    MissingField { field: &'static str },
    #[error("the {field} value must be a list of numbers")]
    NotNumbers { field: &'static str },
    #[error("the {field} value must be a number")]
    NotNumber { field: &'static str },
    #[error("the token count must be a whole number")]
    TokenCount,
    #[error("the token count overflows")]
    TokenOverflow,
    #[error("the embedding index overflows")]
    IndexOverflow,
}
