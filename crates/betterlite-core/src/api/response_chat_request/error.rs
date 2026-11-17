use crate::api::response_chat_history::error::HistoryError;

#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error("{field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("{source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error("{source}")]
    Fields {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
}
