use std::future::{Future, ready};

use async_trait::async_trait;
use betterlite_a2a::completion_bridge::EventSource;
use betterlite_a2a::completion_bridge::completion::{
    CompletionCall, CompletionPort, StreamedChunks,
};
use betterlite_a2a::completion_bridge::dispatch::{A2AProviderDispatch, BridgeEvents};
use betterlite_a2a::completion_bridge::params::BridgeCall;
use betterlite_a2a::completion_bridge::wire::ModelResponse;
use betterlite_a2a::provider_selector::A2AProviderConfig;
use betterlite_a2a::streaming_iterator::{StreamSuccessCall, StreamSuccessDispatch};
use betterlite_core::agents::json::AgentJson;
use betterlite_logging::logging_state::LoggingState;
use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

// The completion bridge runs litellm.acompletion and the provider registry, and this wheel carries neither (B142 owns them).
#[derive(Debug, Error)]
#[error("the A2A completion bridge is not available in the Python SDK")]
pub(crate) struct Unavailable;

pub(crate) struct NoCompletion;

impl CompletionPort for NoCompletion {
    type Error = Unavailable;

    fn complete(
        &self,
        _call: CompletionCall,
    ) -> impl Future<Output = Result<ModelResponse, Self::Error>> + Send {
        ready(Err(Unavailable))
    }

    fn stream(
        &self,
        _call: CompletionCall,
    ) -> impl Future<Output = Result<StreamedChunks<Self::Error>, Self::Error>> + Send {
        ready(Err(Unavailable))
    }
}

pub(crate) struct NoProviders;

impl A2AProviderDispatch for NoProviders {
    type Error = Unavailable;

    fn handle_non_streaming(
        &self,
        _config: A2AProviderConfig,
        _call: &BridgeCall<'_>,
    ) -> impl Future<Output = Result<AgentJson, Self::Error>> + Send {
        ready(Err(Unavailable))
    }

    fn handle_streaming(
        &self,
        _config: A2AProviderConfig,
        _call: &BridgeCall<'_>,
    ) -> impl Future<Output = Result<BridgeEvents<'static, Self::Error>, Self::Error>> + Send {
        ready(Err(Unavailable))
    }
}

// The bridge's synthesized events need identifiers and timestamps, and LiteLLM takes them from the process.
pub(crate) struct SystemSource;

impl EventSource for SystemSource {
    fn next_uuid(&mut self) -> Uuid {
        Uuid::new_v4()
    }

    fn now(&mut self) -> DateTime<Utc> {
        Utc::now()
    }
}

// Python has no logging callbacks registered for these objects, so the streaming accounting ends at B4810's log state.
pub(crate) struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch(&mut self, _logging: &mut LoggingState, _call: StreamSuccessCall) {}
}
