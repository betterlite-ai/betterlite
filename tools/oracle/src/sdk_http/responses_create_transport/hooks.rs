use async_trait::async_trait;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback, AgenticGate, AgenticLoopCall, AgenticOverrides,
};
use betterlite_context::agentic_loop_plan::JsonMap;
use betterlite_core::api::responses_agentic_execution::executor::{
    AnthropicContinuation, ContinuationExecutor, ResponsesContinuation,
};
use betterlite_core::api::responses_create_http::config::Params;
use betterlite_core::api::responses_hook_policy::callbacks::{
    DeploymentHookDispatcher, NamedCallbackResolver, PolicyCallback,
};
use serde_json::value::RawValue;

// The deployment-hook dispatcher behind the hook: the case's patch is the hook's whole answer
pub(super) struct Dispatcher {
    pub(super) patch: Option<Params>,
}

#[async_trait]
impl DeploymentHookDispatcher for Dispatcher {
    type Error = HookFailure;

    async fn pre_call_deployment_hook(
        &self,
        _request: Params,
        _call_type: &str,
    ) -> Result<Option<Params>, Self::Error> {
        Ok(self.patch.clone())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct HookFailure {
    message: String,
}

pub(super) struct NoResolver;

impl NamedCallbackResolver for NoResolver {
    fn resolve(&self, _name: &str) -> Option<PolicyCallback> {
        None
    }
}

// No case reaches a continuation: the scripted hook answers instead of calling the model again
pub(super) struct NoContinuation;

#[async_trait]
impl ContinuationExecutor for NoContinuation {
    type Error = HookFailure;

    async fn responses(&self, _call: &ResponsesContinuation) -> Result<Box<RawValue>, Self::Error> {
        Err(HookFailure {
            message: String::from("a continuation is not expected"),
        })
    }

    async fn anthropic_messages(
        &self,
        _call: &AnthropicContinuation,
    ) -> Result<Box<RawValue>, Self::Error> {
        Err(HookFailure {
            message: String::from("a continuation is not expected"),
        })
    }
}

// The scripted async_should_run_agentic_loop and async_run_agentic_loop: run, then answer the case's result
pub(super) struct Scripted {
    pub(super) result: Option<Box<RawValue>>,
}

#[async_trait]
impl AgenticCallback for Scripted {
    fn callback_name(&self) -> &'static str {
        "scripted"
    }

    fn overrides(&self) -> AgenticOverrides {
        AgenticOverrides {
            should_run: true,
            run_loop: true,
            build_plan: false,
            post_response: false,
            cleanup: false,
        }
    }

    async fn should_run_agentic_loop(
        &self,
        _call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        Ok(AgenticGate {
            run: true,
            tools: JsonMap::new(),
        })
    }

    async fn run_agentic_loop(
        &self,
        _call: &AgenticLoopCall<'_>,
    ) -> Result<Option<Box<RawValue>>, AgenticHookError> {
        Ok(self.result.clone())
    }
}
