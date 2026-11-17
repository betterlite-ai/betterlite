use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::completion_chat_bridge::CompletionEndpoint;
use betterlite_openai::text_completion_request::CompletionRequest;
use betterlite_openai::text_completion_response::CompletionResponse;
use http::HeaderMap;

use crate::dispatch::Dispatch;
use crate::operation::text_stream::TextStreamAnswer;

pub struct TextCompletionRequest {
    pub call_id: String,
    pub model_id: String,
    pub request: Box<CompletionRequest>,
    // the dispatch arm B014 registered takes no Dispatch, so the request hands its entry the pipeline it belongs to
    pub dispatch: Arc<Dispatch>,
    pub calls: Arc<dyn TextCompletionCall>,
}

pub struct TextCompletionCallInput<'call> {
    pub deployment: ChatDeployment,
    pub request: &'call CompletionRequest,
}

// the provider call sits behind this seam, so the pipeline stays in betterlite-core
#[async_trait]
pub trait TextCompletionCall: Send + Sync {
    async fn complete(
        &self,
        call: TextCompletionCallInput<'_>,
    ) -> Result<TextCompletionResult, TextCompletionFailure>;

    // where main.py sends this deployment: the text endpoint, or a chat endpoint whose answer converts
    async fn endpoint(&self, _deployment: &ChatDeployment) -> CompletionEndpoint {
        CompletionEndpoint::Text
    }

    async fn stream(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextStreamAnswer, TextCompletionFailure> {
        Err(TextCompletionFailure::internal("no chat bridge here"))
    }

    async fn chat(
        &self,
        _call: TextCompletionCallInput<'_>,
        _mode: ChatCallMode,
    ) -> Result<ChatCallAnswer, TextCompletionFailure> {
        Err(TextCompletionFailure::internal("no chat bridge here"))
    }
}

pub enum ChatCallMode {
    Buffered,
    Streamed,
}

pub enum ChatCallAnswer {
    Buffered(TextCompletionResult),
    Streamed(TextStreamAnswer),
}

const MISMATCHED: &str = "this completion port answered a different operation";

impl ChatCallAnswer {
    pub fn buffered(self) -> Result<TextCompletionResult, TextCompletionFailure> {
        match self {
            Self::Buffered(answer) => Ok(answer),
            Self::Streamed(_) => Err(TextCompletionFailure::internal(MISMATCHED)),
        }
    }

    pub fn streamed(self) -> Result<TextStreamAnswer, TextCompletionFailure> {
        match self {
            Self::Streamed(answer) => Ok(answer),
            Self::Buffered(_) => Err(TextCompletionFailure::internal(MISMATCHED)),
        }
    }
}

pub struct TextCompletionResult {
    pub response: Box<CompletionResponse>,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}

// the status, type, param and message the route answers with; a null type is an empty string here
#[derive(Debug)]
pub struct TextCompletionFailure {
    pub status: u16,
    pub error_type: Option<String>,
    pub param: Option<String>,
    pub message: String,
}

impl TextCompletionFailure {
    // a failure the route writes itself, which names no parameter
    pub fn new(status: u16, error_type: Option<&str>, message: String) -> Self {
        Self {
            status,
            error_type: error_type.map(str::to_owned),
            param: None,
            message,
        }
    }

    // a 500 the route writes itself, which names no parameter
    #[must_use]
    pub fn internal(message: &str) -> Self {
        Self {
            status: 500,
            error_type: Some(String::from("internal_server_error")),
            param: None,
            message: message.to_owned(),
        }
    }
}
