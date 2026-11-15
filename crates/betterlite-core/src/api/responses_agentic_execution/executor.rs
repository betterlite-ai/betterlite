use super::LoggingView;
use crate::api::responses_hook_policy::agentic::AgenticLoopSettings;
use crate::api::responses_hook_policy::safety::ApiSurface;
use async_trait::async_trait;
use betterlite_context::agentic_callback::{AgenticCallback, AgenticGate, AgenticLogging};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage, JsonMap};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use std::error::Error;
#[derive(Clone, Debug)]
pub struct ResponsesContinuation {
    pub model: String,
    pub input: Vec<FollowupMessage>,
    pub params: IndexMap<String, Json>,
    pub kwargs: IndexMap<String, Json>,
}
#[derive(Clone, Debug)]
pub struct AnthropicContinuation {
    pub model: String,
    pub messages: Vec<FollowupMessage>,
    pub max_tokens: u64,
    pub stream: bool,
    pub params: IndexMap<String, Json>,
}
#[async_trait]
pub trait ContinuationExecutor: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    async fn responses(&self, call: &ResponsesContinuation) -> Result<Box<RawValue>, Self::Error>;
    async fn anthropic_messages(
        &self,
        call: &AnthropicContinuation,
    ) -> Result<Box<RawValue>, Self::Error>;
}
#[derive(Clone)]
pub struct CompletionCall<'call> {
    pub response: &'call RawValue,
    pub model: &'call str,
    pub messages: &'call [FollowupMessage],
    pub optional_params: &'call JsonMap,
    pub logging: Option<LoggingView<'call>>,
    pub stream: bool,
    pub custom_llm_provider: &'call str,
    pub kwargs: &'call IndexMap<String, Json>,
    pub api_surface: ApiSurface,
}
pub(crate) struct LoopState<'state> {
    pub(crate) settings: &'state AgenticLoopSettings,
    pub(crate) logging: Option<AgenticLogging>,
    pub(crate) surface_kwargs: &'state JsonMap,
    pub(crate) hook_kwargs: &'state JsonMap,
    pub(crate) provider_kwargs: &'state IndexMap<String, Json>,
}
pub(crate) struct CallbackRun<'run> {
    pub(crate) call: CompletionCall<'run>,
    pub(crate) callback: &'run dyn AgenticCallback,
    pub(crate) gate: &'run AgenticGate,
    pub(crate) tool_calls: &'run Json,
    pub(crate) settings: &'run AgenticLoopSettings,
    pub(crate) fingerprint: &'run str,
    pub(crate) hook_kwargs: &'run JsonMap,
    pub(crate) provider_kwargs: &'run IndexMap<String, Json>,
    pub(crate) logging: Option<&'run AgenticLogging>,
}
pub(crate) struct PlanRun<'run, 'plan> {
    pub(crate) run: &'run CallbackRun<'run>,
    pub(crate) plan: &'plan AgenticLoopPlan,
}
