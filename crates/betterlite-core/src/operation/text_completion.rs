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

    // the text route's chat-deployment branch: the answer converts back to a text completion
    async fn complete_chat(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextCompletionResult, TextCompletionFailure> {
        Err(not_built_chat())
    }

    async fn stream(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextStreamAnswer, TextCompletionFailure> {
        Err(not_built_chat())
    }

    // the text route's streamed chat-deployment branch: chat chunks for B5366's text wrapper
    async fn stream_chat(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextStreamAnswer, TextCompletionFailure> {
        Err(not_built_chat())
    }
}

fn not_built_chat() -> TextCompletionFailure {
    TextCompletionFailure {
        status: 500,
        error_type: Some(String::from("internal_server_error")),
        param: None,
        message: String::from("this completion port has no chat deployment bridge"),
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
