use async_trait::async_trait;
use serde_json::value::RawValue;

use crate::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage, JsonMap};

use error::AgenticHookError;

pub mod error;
mod tests;

// clippy's two-bool limit counts the primitive spelling only, so the override flags share this alias
pub type AgenticFlag = bool;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgenticOverrides {
    pub should_run: AgenticFlag,
    pub run_loop: AgenticFlag,
    pub build_plan: AgenticFlag,
    pub post_response: AgenticFlag,
    pub cleanup: AgenticFlag,
}

#[derive(Clone, Debug, Default)]
pub struct AgenticGate {
    pub run: bool,
    pub tools: JsonMap,
}

#[derive(Clone, Copy, Debug)]
pub struct AgenticCall<'call> {
    pub response: &'call RawValue,
    pub model: &'call str,
    pub messages: &'call [FollowupMessage],
    pub tools: Option<&'call [FollowupMessage]>,
    pub stream: bool,
    pub custom_llm_provider: &'call str,
    pub kwargs: &'call JsonMap,
}

#[derive(Clone, Debug)]
pub struct AgenticLogging {
    pub call_id: Option<String>,
    pub agentic_loop_params: Option<JsonMap>,
}

#[derive(Clone, Copy, Debug)]
pub struct AgenticLoopCall<'call> {
    pub tools: &'call JsonMap,
    pub model: &'call str,
    pub messages: &'call [FollowupMessage],
    pub response: &'call RawValue,
    pub optional_params: &'call JsonMap,
    pub logging: Option<&'call AgenticLogging>,
    pub stream: bool,
    pub kwargs: &'call JsonMap,
}

#[async_trait]
pub trait AgenticCallback: Send + Sync {
    fn callback_name(&self) -> &str;

    fn overrides(&self) -> AgenticOverrides;

    async fn should_run_agentic_loop(
        &self,
        _call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        Ok(AgenticGate {
            run: false,
            tools: JsonMap::new(),
        })
    }

    async fn build_agentic_loop_plan(
        &self,
        _call: &AgenticLoopCall<'_>,
    ) -> Result<AgenticLoopPlan, AgenticHookError> {
        Ok(AgenticLoopPlan::default())
    }

    // CustomLogger's body is only a docstring, so None leaves the loop's own response in place
    async fn run_agentic_loop(
        &self,
        _call: &AgenticLoopCall<'_>,
    ) -> Result<Option<Box<RawValue>>, AgenticHookError> {
        Ok(None)
    }

    async fn post_agentic_loop_response(
        &self,
        response: Box<RawValue>,
        _plan: &AgenticLoopPlan,
        _kwargs: &JsonMap,
    ) -> Result<Box<RawValue>, AgenticHookError> {
        Ok(response)
    }

    async fn agentic_loop_cleanup(
        &self,
        _plan: &AgenticLoopPlan,
        _kwargs: &JsonMap,
    ) -> Result<(), AgenticHookError> {
        Ok(())
    }
}
