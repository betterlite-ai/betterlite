use betterlite_openai::openai_base_config::Headers;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RecordsError {
    #[error("the {field} value cannot be decoded")]
    Value {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {field} timings cannot be divided into seconds")]
    Timing { field: &'static str },
    #[error("the {field} text is not a string")]
    Text { field: &'static str },
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct SonioxException {
    pub message: String,
    pub status_code: i64,
    pub headers: Option<Headers>,
}
