use betterlite_core::agents::provider::a2a::JsonRpcId;
use thiserror::Error;

use crate::client::records::JSONRPC_VERSION;
use crate::client::records::compat::SendMessageRequest;
use crate::client::records::compat::{TaskArtifactUpdate, TaskStatusUpdate};
use crate::client::records::core::{CoreSendMessageRequest, CoreStreamEvent};
use crate::client::records::response::{SendMessageResponse, StreamResult};

pub mod messages;
pub mod parts;
pub mod request;
pub mod response;

#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("the A2A file part bytes are not base64")]
    FileBytes(#[source] base64::DecodeError),
    #[error("the A2A data part marked data_part_compat carries no value")]
    MissingDataValue,
}

pub fn to_core_send_message_request(
    request: SendMessageRequest,
) -> Result<CoreSendMessageRequest, ConvertError> {
    Ok(CoreSendMessageRequest {
        message: messages::to_core_message(request.params.message)?,
        configuration: request
            .params
            .configuration
            .map(request::to_core_configuration),
        metadata: request.params.metadata,
    })
}

pub fn to_compat_stream_response(
    event: CoreStreamEvent,
    request_id: JsonRpcId,
) -> Result<SendMessageResponse, ConvertError> {
    let result = match event {
        CoreStreamEvent::Message(message) => {
            StreamResult::Message(messages::to_compat_message(&message)?)
        }
        CoreStreamEvent::Task(task) => StreamResult::Task(response::to_compat_task(&task)?),
        CoreStreamEvent::StatusUpdate(update) => {
            let status = response::compat_status_of(update.status.as_ref())?;
            let is_final = response::is_final(&status);
            StreamResult::StatusUpdate(TaskStatusUpdate {
                status,
                is_final,
                metadata: update.metadata,
                task_id: update.task_id,
                context_id: update.context_id.unwrap_or_default(),
            })
        }
        CoreStreamEvent::ArtifactUpdate(update) => {
            StreamResult::ArtifactUpdate(TaskArtifactUpdate {
                append: update.append,
                last_chunk: update.last_chunk,
                metadata: update.metadata,
                task_id: update.task_id,
                context_id: update.context_id.unwrap_or_default(),
                artifact: response::to_compat_artifact(&update.artifact)?,
            })
        }
    };
    Ok(SendMessageResponse {
        jsonrpc: JSONRPC_VERSION.to_owned(),
        id: request_id,
        result,
    })
}
