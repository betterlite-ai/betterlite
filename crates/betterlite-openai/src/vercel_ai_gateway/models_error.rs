use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelsError {
    #[error("Failed to get models: {text}")]
    Status { code: u16, text: String },
    #[error("the model list reply is not valid")]
    Shape(#[source] serde_json::Error),
}

impl ModelsError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Status { code, .. } => *code,
            Self::Shape(_) => 502,
        }
    }
}
