use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::builders::{
    InterpreterInput, build_interpreter_logs,
};
use betterlite_openai::api::responses_shapes::{
    CodeInterpreterCallType, OutputCallStatus, OutputCodeInterpreterCall,
    OutputCodeInterpreterCallLog,
};

use super::error::StreamStateError;
use super::state::StreamState;
use super::wire;
use crate::response_content::wire::json_text;

pub fn build_code_interpreter_results(
    state: &StreamState,
) -> Result<Vec<OutputCodeInterpreterCall>, StreamStateError> {
    let mut results = Vec::with_capacity(state.tool_results.len());
    for result in &state.tool_results {
        let Some(call) = interpreter_call(state, result)? else {
            continue;
        };
        results.push(call);
    }
    Ok(results)
}

fn interpreter_call(
    state: &StreamState,
    result: &Json,
) -> Result<Option<OutputCodeInterpreterCall>, StreamStateError> {
    let Json::Object(fields) = result else {
        return Ok(None);
    };
    if fields.get("type").and_then(json_text) != Some("bash_code_execution_tool_result") {
        return Ok(None);
    }
    let id = match fields.get("tool_use_id") {
        None => String::new(),
        Some(Json::Text(text)) => text.clone(),
        Some(_) => {
            return Err(StreamStateError::FieldType {
                field: "tool_use_id",
            });
        }
    };
    let content = fields.get("content").cloned().unwrap_or(Json::Null);
    let outputs = interpreter_outputs(&content)?;
    let code = server_tool_command(state.server_tool_inputs.get(&id))?;
    Ok(Some(OutputCodeInterpreterCall {
        r#type: CodeInterpreterCallType::CodeInterpreterCall,
        id,
        code,
        container_id: state.container_id.clone(),
        status: OutputCallStatus::Completed,
        outputs,
        extra: BTreeMap::new(),
    }))
}

fn interpreter_outputs(
    content: &Json,
) -> Result<Option<Vec<OutputCodeInterpreterCallLog>>, StreamStateError> {
    let input = wire::decode::<InterpreterInput>(content)
        .map_err(|source| StreamStateError::Decode { source })?;
    Ok(build_interpreter_logs(&input))
}

fn server_tool_command(input: Option<&Json>) -> Result<Option<String>, StreamStateError> {
    let Some(Json::Object(fields)) = input else {
        return Ok(Some(String::new()));
    };
    match fields.get("command") {
        None => Ok(Some(String::new())),
        Some(Json::Null) => Ok(None),
        Some(Json::Text(command)) => Ok(Some(command.clone())),
        Some(_) => Err(StreamStateError::FieldType { field: "command" }),
    }
}
