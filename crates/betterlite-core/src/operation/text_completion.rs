use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::text_completion_request::CompletionRequest;
use betterlite_openai::text_completion_response::CompletionResponse;
use http::HeaderMap;

use crate::dispatch::Dispatch;

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
}

pub struct TextCompletionResult {
    pub response: Box<CompletionResponse>,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}

// the status, type, param and message the route answers with; the dispatch hands them on as its Rejected answer
#[derive(Debug)]
pub struct TextCompletionFailure {
    pub status: u16,
    pub error_type: String,
    pub param: Option<String>,
    pub message: String,
}
