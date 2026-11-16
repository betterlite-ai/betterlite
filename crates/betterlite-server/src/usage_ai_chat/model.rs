use std::future::Future;

use futures_util::Stream;
use thiserror::Error;

use super::messages::ChatMessage;
use super::tools::ToolDefinition;

pub struct ModelRequest<'a> {
    pub model: &'a str,
    pub messages: &'a [ChatMessage],
    pub tools: Option<&'a [ToolDefinition]>,
    pub temperature: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelReply {
    pub content: Option<String>,
    pub tool_calls: Vec<ModelToolCall>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModelError {
    #[error("the completion call failed: {message}")]
    Completion { message: String },
    #[error("the completion stream failed: {message}")]
    Stream { message: String },
}

pub trait UsageChatModel: Sync {
    fn complete(
        &self,
        request: &ModelRequest<'_>,
    ) -> impl Future<Output = Result<ModelReply, ModelError>> + Send;

    fn stream(
        &self,
        request: &ModelRequest<'_>,
    ) -> impl Future<
        Output = Result<impl Stream<Item = Result<String, ModelError>> + Send, ModelError>,
    > + Send;
}
