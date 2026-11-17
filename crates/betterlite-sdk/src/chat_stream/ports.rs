use std::sync::Arc;

use async_trait::async_trait;
use betterlite_core::routes::CallType;
use betterlite_core::rules::{RuleDecision, RuleEvaluator};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_logging::logging_streams::assembly::StreamBuild;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::stream_chunk_shaping::creation::{StreamInstant, system_instant};
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CorrelationRestore;
use crate::call_setup::hooks::HookError;

// Which httpx failure ended the stream; only read/protocol errors after a finish reason end it cleanly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceFailure {
    Timeout,
    Read,
    Protocol,
    Other,
}

// A failing source read, already mapped to betterlite's error taxonomy by the transport.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct StreamSourceError {
    pub failure: SourceFailure,
    pub class: ErrorClass,
    pub message: String,
}

// The provider stream CustomStreamWrapper iterates: one async next plus a close.
#[async_trait]
pub trait ChunkSource: Send + Sync {
    async fn next_chunk(&mut self) -> Result<Option<ProviderChunk>, StreamSourceError>;
    async fn close(&mut self) -> Result<(), StreamSourceError>;
}

// time.time() for the duration check and completion_start_time; tests advance it.
pub trait StreamClock: Send + Sync {
    fn now(&self) -> StreamInstant;
}

pub struct SystemClock;

impl StreamClock for SystemClock {
    fn now(&self) -> StreamInstant {
        system_instant()
    }
}

// B4300's cache port: only the finished response is stored, and never on a cache hit.
#[async_trait]
pub trait StreamCache: Send + Sync {
    async fn store(&self, response: Json);
}

// CustomLogger.async_post_call_streaming_deployment_hook, which DeploymentHook has no method for yet.
#[async_trait]
pub trait StreamingDeploymentHook: Send + Sync {
    async fn stream_chunk(
        &self,
        request: &RequestMap,
        call_type: Option<CallType>,
        chunk: &StreamChunk,
    ) -> Result<Option<StreamChunk>, HookError>;
}

pub struct StreamFailure {
    pub class: ErrorClass,
    pub message: String,
}

// B252's stream logging: the three stream_chunk_builder call sites stay behind completed and partial_usage.
pub trait StreamLogging: Send + Sync {
    fn started(&self, at: StreamInstant);
    fn log_chunk(&self, chunk: &StreamChunk, cache_hit: bool);
    fn completed(&self, chunks: &[StreamChunk]) -> StreamBuild;
    fn partial_usage(&self, chunks: &[StreamChunk]);
    fn log_failure(&self, failure: &StreamFailure);
    fn log_success(&self, response: Option<Json>, cache_hit: bool);
}

pub struct ChatStreamPorts<'ports> {
    pub clock: &'ports dyn StreamClock,
    pub logging: &'ports dyn StreamLogging,
    pub cache: Option<&'ports dyn StreamCache>,
    pub hooks: &'ports [Arc<dyn StreamingDeploymentHook>],
    pub request_data: &'ports RequestMap,
    pub call_type: &'ports str,
    pub rules: &'ports (dyn RuleEvaluator + Sync),
    pub restore: &'ports (dyn CorrelationRestore + Sync),
    pub correlation: &'ports mut CorrelationContext,
    pub redactor: &'ports Redactor,
}

// betterlite-core's evaluate is generic over the evaluator, so a rule port needs this thin adapter.
pub(super) struct SharedRules<'rules>(pub(super) &'rules (dyn RuleEvaluator + Sync));

impl RuleEvaluator for SharedRules<'_> {
    fn evaluate(&self, index: usize, input: &str) -> Option<RuleDecision> {
        self.0.evaluate(index, input)
    }
}
