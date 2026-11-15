use std::error::Error;

use async_trait::async_trait;
use betterlite_context::agentic_callback::{AgenticCallback, AgenticGate, AgenticLogging};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage, JsonMap};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::value::RawValue;

use super::LoggingView;
use crate::api::responses_hook_policy::agentic::AgenticLoopSettings;
use crate::api::responses_hook_policy::safety::ApiSurface;

// one follow-up Responses call, built from the plan's patch over the request's own parameters
#[derive(Clone, Debug)]
pub struct ResponsesContinuation {
    pub model: String,
    pub input: Vec<FollowupMessage>,
    pub params: IndexMap<String, Json>,
    pub kwargs: IndexMap<String, Json>,
}

// one follow-up Anthropic Messages call: the merged mapping split into named fields and the rest
#[derive(Clone, Debug)]
pub struct AnthropicContinuation {
    pub model: String,
    pub messages: Vec<FollowupMessage>,
    pub max_tokens: u64,
    pub stream: bool,
    pub params: IndexMap<String, Json>,
}

// the family composition supplies the recursive call; this module never reaches its callers
#[async_trait]
pub trait ContinuationExecutor: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn responses(&self, call: &ResponsesContinuation) -> Result<Box<RawValue>, Self::Error>;

    async fn anthropic_messages(
        &self,
        call: &AnthropicContinuation,
    ) -> Result<Box<RawValue>, Self::Error>;
}

// everything the shared dispatcher is handed for one completion
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

// the loop-wide state built once per request
pub(crate) struct LoopState<'state> {
    pub(crate) settings: &'state AgenticLoopSettings,
    pub(crate) logging: Option<AgenticLogging>,
    pub(crate) surface_kwargs: &'state JsonMap,
    pub(crate) hook_kwargs: &'state JsonMap,
    pub(crate) provider_kwargs: &'state IndexMap<String, Json>,
}

// one callback's run, bundled because the plans and the tests each read most of it
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

// a callback's run together with the plan it built
pub(crate) struct PlanRun<'run, 'plan> {
    pub(crate) run: &'run CallbackRun<'run>,
    pub(crate) plan: &'plan AgenticLoopPlan,
}
