use serde::Serialize;
use serde_json::Value;

use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, AgenticLoopRequestPatch, JsonMap};

use crate::code_execution_records::CallDialect;
use crate::code_execution_records::followup_filtering;

use super::followup;
use super::records::{CodeInterpreterCall, followup_messages};
use super::request::PlanRequest;

// LiteLLM's plan metadata object: the tool type, the sandbox key, the scope flag and each call record
#[derive(Serialize)]
struct PlanMetadata<'plan> {
    tool_type: &'static str,
    sandbox_key: &'plan str,
    is_session_scoped: bool,
    code_interpreter_calls: &'plan [CodeInterpreterCall],
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<&'plan str>,
}

pub fn metadata(
    sandbox_key: Option<&str>,
    is_session_scoped: bool,
    code_interpreter_calls: &[CodeInterpreterCall],
    response_format: Option<&str>,
) -> Result<JsonMap, serde_json::Error> {
    let fields = PlanMetadata {
        tool_type: "code_interpreter",
        sandbox_key: sandbox_key.unwrap_or_default(),
        is_session_scoped,
        code_interpreter_calls,
        response_format,
    };
    let serialized = serde_json::to_string(&fields)?;
    serde_json::from_str(&serialized)
}

pub fn assemble(
    request: &PlanRequest<'_>,
    messages: &[Value],
    dialect: CallDialect,
    metadata: JsonMap,
) -> Result<AgenticLoopPlan, serde_json::Error> {
    let request_patch = AgenticLoopRequestPatch {
        model: Some(request.model.to_owned()),
        messages: Some(followup_messages(messages)?),
        tools: followup::tools(request.optional_params.get("tools"), dialect)?,
        max_tokens: None,
        optional_params: followup::raw_map(&followup_filtering::optional_params(
            request.optional_params,
        ))?,
        kwargs: followup::raw_map(&followup_filtering::agentic_loop_kwargs(request.kwargs))?,
    };
    Ok(AgenticLoopPlan {
        run_agentic_loop: true,
        request_patch: Some(request_patch),
        response_override: None,
        terminate: false,
        stop_reason: None,
        metadata,
    })
}
