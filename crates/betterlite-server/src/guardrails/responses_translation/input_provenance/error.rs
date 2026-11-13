use betterlite_core::api::response_chat_history::error::HistoryError;
use betterlite_core::api::responses_input_messages::error::InputMessagesError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InputProvenanceError {
    #[error("the Responses input converter failed")]
    Conversion {
        #[source]
        source: InputMessagesError,
    },
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error("a rewritten row is not a chat message")]
    ChatRow {
        #[source]
        source: serde_json::Error,
    },
    #[error("a raw Responses input item is not plain JSON")]
    InputItem {
        #[source]
        source: serde_json::Error,
    },
}
