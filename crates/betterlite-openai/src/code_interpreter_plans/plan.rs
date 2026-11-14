use serde_json::Value;
use serde_json::value::to_raw_value;

use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, AgenticLoopRequestPatch, JsonMap};

use crate::code_execution_records::CallDialect;

use super::followup;
use super::records::{CodeInterpreterCall, followup_messages};
use super::request::PlanRequest;

pub fn metadata(
    sandbox_key: Option<&str>,
    is_session_scoped: bool,
    code_interpreter_calls: &[CodeInterpreterCall],
    response_format: Option<&str>,
) -> Result<JsonMap, serde_json::Error> {
    let mut metadata = JsonMap::new();
    metadata.insert("tool_type".to_owned(), to_raw_value("code_interpreter")?);
    metadata.insert(
        "sandbox_key".to_owned(),
        to_raw_value(sandbox_key.unwrap_or_default())?,
    );
    metadata.insert(
        "is_session_scoped".to_owned(),
        to_raw_value(&is_session_scoped)?,
    );
    metadata.insert(
        "code_interpreter_calls".to_owned(),
        to_raw_value(code_interpreter_calls)?,
    );
    if let Some(response_format) = response_format {
        metadata.insert("response_format".to_owned(), to_raw_value(response_format)?);
    }
    Ok(metadata)
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
        optional_params: followup::optional_params(request.optional_params)?,
        kwargs: followup::kwargs(request.kwargs)?,
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
