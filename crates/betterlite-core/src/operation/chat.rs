use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_models::controls::control_options::{ControlOptions, InvalidControlOption};
use betterlite_openai::chat::ChatRequest as ClientChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatError;
use futures_util::stream::BoxStream;
use http::HeaderMap;
use thiserror::Error;

pub struct ChatRequest {
    pub call_id: String,
    pub model_id: String,
    pub request: Box<ClientChatRequest>,
    pub calls: Arc<dyn ChatCall>,
}

// the existing chat implementation sits behind this seam, so the pipeline stays in betterlite-core
#[async_trait]
pub trait ChatCall: Send + Sync {
    fn prepare(&self, request: &ClientChatRequest) -> Result<ControlOptions, ChatFailure>;
    async fn complete(
        &self,
        call: ChatCallInput<'_>,
        control: ControlOptions,
    ) -> Result<ChatCompletionAnswer, ChatFailure>;
    async fn stream(
        &self,
        call: ChatCallInput<'_>,
        control: ControlOptions,
    ) -> Result<ChatStreamAnswer, ChatFailure>;
}

pub struct ChatCallInput<'call> {
    pub deployment: ChatDeployment,
    pub request: &'call ClientChatRequest,
}

// the completion body as the provider framework writes it, so core never names a provider crate type
pub struct ChatCompletionAnswer {
    pub body: Vec<u8>,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}

pub struct ChatStreamAnswer {
    pub body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}

pub enum ChatResult {
    Completion(ChatCompletionAnswer),
    Stream(ChatStreamAnswer),
}

#[derive(Debug, Error)]
pub enum ChatFailure {
    #[error(transparent)]
    Route(#[from] ChatError),
    #[error("{message}")]
    Internal { message: String },
    #[error(transparent)]
    InvalidControl {
        #[from]
        source: InvalidControlOption,
    },
    #[error("the provider answered a different operation than the one sent")]
    Mismatched,
}
