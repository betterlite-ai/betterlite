use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::reply::{ReplyFunction, ReplyToolCall};
use crate::databricks_chat::response::wire::DatabricksToolCall;
use crate::ollama_chat::ids::IdSource;

const CUSTOM: &str = "custom";
const JSON_TOOL_CALL: &str = "json_tool_call";
const MULTI_TOOL: &str = "multi_tool_use.parallel";

// The arguments of a multi_tool_use.parallel call, read with ["tool_uses"]
#[derive(Deserialize)]
struct ParallelArguments {
    tool_uses: Vec<ToolUse>,
}

#[derive(Deserialize)]
struct ToolUse {
    recipient_name: String,
    // the arguments of the hidden call, encoded again as they came
    parameters: Json,
}

// Message(tool_calls=...) keeps a non-empty list and turns an empty one into null
pub(super) fn normalize(
    tool_calls: Option<&[DatabricksToolCall]>,
    ids: &dyn IdSource,
) -> Result<Option<Vec<ReplyToolCall>>, DatabricksChatError> {
    let Some(calls) = tool_calls.filter(|calls| !calls.is_empty()) else {
        return Ok(None);
    };
    let mut normalized = Vec::with_capacity(calls.len());
    for call in calls {
        if call.kind.as_deref() == Some(CUSTOM) || call.custom.is_some() {
            return Err(DatabricksChatError::Shape {
                message: String::from("a custom tool call in a Databricks reply is not supported"),
            });
        }
        normalized.push(ReplyToolCall {
            extra: call.extra.clone(),
            function: ReplyFunction {
                arguments: call.function.arguments_text()?,
                name: call.function.name.clone(),
            },
            id: call.id.clone().unwrap_or_else(|| ids.next_id()),
            kind: call
                .kind
                .clone()
                .unwrap_or_else(|| String::from("function")),
        });
    }
    Ok(Some(normalized))
}

// _handle_invalid_parallel_tool_calls: a call whose arguments are not JSON leaves the list as it came
pub(super) fn repair(calls: &[ReplyToolCall]) -> Result<Vec<ReplyToolCall>, DatabricksChatError> {
    let mut repaired = Vec::with_capacity(calls.len());
    for call in calls {
        if serde_json::from_str::<IgnoredAny>(&call.function.arguments).is_err() {
            return Ok(calls.to_vec());
        }
        if call.function.name.as_deref() != Some(MULTI_TOOL) {
            repaired.push(call.clone());
            continue;
        }
        let parallel: ParallelArguments =
            serde_json::from_str(&call.function.arguments).map_err(DatabricksChatError::Decode)?;
        if parallel.tool_uses.is_empty() {
            repaired.push(call.clone());
        }
        for (position, tool_use) in parallel.tool_uses.iter().enumerate() {
            let recipient = tool_use.recipient_name.as_str();
            repaired.push(ReplyToolCall {
                extra: Object::new(),
                function: ReplyFunction {
                    arguments: serde_json::to_string(&tool_use.parameters)
                        .map_err(DatabricksChatError::Encode)?,
                    name: Some(
                        recipient
                            .strip_prefix("functions.")
                            .unwrap_or(recipient)
                            .to_owned(),
                    ),
                },
                id: format!("{}_{position}", call.id),
                kind: String::from("function"),
            });
        }
    }
    Ok(repaired)
}

// _should_convert_tool_call_to_json_mode: one json_tool_call with arguments becomes the content
pub(super) fn json_mode_content(calls: &[ReplyToolCall], json_mode: bool) -> Option<String> {
    let [call] = calls else {
        return None;
    };
    let converts = json_mode
        && call.function.name.as_deref() == Some(JSON_TOOL_CALL)
        && !call.function.arguments.is_empty();
    converts.then(|| call.function.arguments.clone())
}
