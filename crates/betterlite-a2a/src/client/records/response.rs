use super::compat::{Message, Task, TaskArtifactUpdate, TaskStatusUpdate};
use betterlite_core::agents::provider::a2a::JsonRpcId;
use serde::Serialize;
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[rustfmt::skip]
pub struct SendMessageResponse { pub jsonrpc: String, pub id: JsonRpcId, pub result: StreamResult }
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)] #[rustfmt::skip]
pub enum StreamResult { Message(Message), Task(Task), StatusUpdate(TaskStatusUpdate), ArtifactUpdate(TaskArtifactUpdate) }
