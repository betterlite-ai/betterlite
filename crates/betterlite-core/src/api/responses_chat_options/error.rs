use thiserror::Error;

use crate::api::responses_input_messages::error::InputMessagesError;
use crate::api::responses_tool_namespaces::error::NamespaceToolsError;

#[derive(Debug, Error)]
pub enum ChatOptionsError {
    #[error("the {field} field cannot hold this value")]
    WrongType { field: &'static str },
    #[error("the chat request cannot be encoded")]
    Encode {
        #[from]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Tools(#[from] NamespaceToolsError),
    #[error(transparent)]
    Messages(#[from] InputMessagesError),
}
