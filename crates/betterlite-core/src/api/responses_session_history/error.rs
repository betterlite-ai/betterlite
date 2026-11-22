use std::error::Error as StdError;

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_error::error::ErrorClass;
use thiserror::Error;

use crate::api::responses_input_messages::error::InputMessagesError;
use crate::api::responses_session_history::decode::SessionLogJsonError;
use crate::api::responses_tool_repair::error::RepairError;

#[derive(Debug, Error)]
pub enum SessionHistoryError<Cold>
where
    Cold: StdError + 'static,
{
    #[error("the spend log session query failed")]
    Query(#[source] TableError),
    #[error("a stored proxy_server_request could not be read")]
    StoredRequest(#[source] SessionLogJsonError),
    #[error("the stored {column} column does not have the type LiteLLM declares")]
    Column { column: &'static str },
    #[error("a stored spend log does not have the type LiteLLM declares")]
    Record(#[from] serde_json::Error),
    #[error("the cold storage read failed")]
    ColdStorage(#[source] Cold),
    #[error(transparent)]
    Input(#[from] InputMessagesError),
    #[error(transparent)]
    Repair(#[from] RepairError),
    // LiteLLM raises BadRequestError when neither the session nor the request leaves a message
    #[error("unable to create messages for the completion request")]
    NoMessages(ErrorClass),
}
