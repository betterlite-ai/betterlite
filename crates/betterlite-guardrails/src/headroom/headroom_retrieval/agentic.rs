use async_trait::async_trait;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback, AgenticGate, AgenticLoopCall, AgenticOverrides,
};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, JsonMap};
use serde_json::value::to_raw_value;

use crate::headroom::headroom_compression::HeadroomGuardrail;

use super::error::RetrievalError;
use super::plan;
use super::response::ModelResponse;
use super::tools;

const CALLBACK_NAME: &str = "headroom";
const TOOL_CALLS_KEY: &str = "tool_calls";

#[async_trait]
impl AgenticCallback for HeadroomGuardrail {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    fn overrides(&self) -> AgenticOverrides {
        AgenticOverrides {
            should_run: true,
            run_loop: false,
            build_plan: true,
            post_response: false,
            cleanup: false,
        }
    }

    async fn should_run_agentic_loop(
        &self,
        call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        if !tools::request_holds_retrieve_tool(call.tools) {
            return Ok(AgenticGate::default());
        }
        let response = ModelResponse::parse(call.response).map_err(|error| hook_error(&error))?;
        let calls = tools::retrieve_calls(&response.calls);
        if calls.is_empty() {
            return Ok(AgenticGate::default());
        }
        let mut gate_tools = JsonMap::new();
        let encoded =
            to_raw_value(&calls).map_err(|error| hook_error(&RetrievalError::from(error)))?;
        gate_tools.insert(TOOL_CALLS_KEY.to_owned(), encoded);
        Ok(AgenticGate {
            run: true,
            tools: gate_tools,
        })
    }

    async fn build_agentic_loop_plan(
        &self,
        call: &AgenticLoopCall<'_>,
    ) -> Result<AgenticLoopPlan, AgenticHookError> {
        plan::build(self, call)
            .await
            .map_err(|error| hook_error(&error))
    }
}

fn hook_error(error: &RetrievalError) -> AgenticHookError {
    AgenticHookError::Failed {
        callback: CALLBACK_NAME.to_owned(),
        message: error.to_string(),
    }
}
