use std::error::Error;
use std::future::Future;

use futures_util::Stream;
use thiserror::Error;

use super::messages::ChatMessage;
use super::tools::ToolDefinition;

pub struct ModelRequest<'text> {
    pub model: &'text str,
    pub messages: &'text [ChatMessage],
    pub tools: Option<&'text [ToolDefinition]>,
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

#[derive(Debug, Error)]
pub enum ModelError<Fault>
where
    Fault: Error + 'static,
{
    #[error("the completion call failed: {source}")]
    Completion {
        #[source]
        source: Fault,
    },
    #[error("the completion stream failed: {source}")]
    Stream {
        #[source]
        source: Fault,
    },
}

pub trait UsageChatModel: Sync {
    type Error: Error + Send + Sync + 'static;

    fn complete(
        &self,
        request: &ModelRequest<'_>,
    ) -> impl Future<Output = Result<ModelReply, Self::Error>> + Send;

    fn stream(
        &self,
        request: &ModelRequest<'_>,
    ) -> impl Future<
        Output = Result<impl Stream<Item = Result<String, Self::Error>> + Send, Self::Error>,
    > + Send;
}
