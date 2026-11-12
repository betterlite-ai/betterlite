use crate::gemini_response_parts::projection::{CallMode, Part, PartKind, PartsError, ServerTool};
use betterlite_models::decisions::Json;
use betterlite_prompts::gemini_tool_invokes::signatures::encode_tool_call_id;
use indexmap::IndexMap;
use serde::Serialize;
use uuid::Uuid;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionChunk {
    pub name: String,
    pub arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<SignatureFields>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignatureFields {
    pub thought_signature: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolCallChunk {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: PartKind,
    pub function: FunctionChunk,
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<SignatureFields>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerToolInvocation {
    pub tool_type: Option<String>,
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_thought_signature: Option<String>,
}
type TransformedParts = (Option<FunctionChunk>, Option<Vec<ToolCallChunk>>, u32);

pub(crate) fn transform(
    parts: &[Part],
    cumulative: u32,
    mode: CallMode,
    ids: &mut dyn FnMut() -> Uuid,
) -> Result<TransformedParts, PartsError> {
    let mut function = None;
    let mut tools = Vec::new();
    let mut index = cumulative;
    for part in parts {
        let Some(call) = &part.function_call else {
            continue;
        };
        let chunk = FunctionChunk {
            name: call.name.clone(),
            arguments: arguments(call.args.as_ref())?,
            provider_specific_fields: match mode {
                CallMode::LegacyFunction => signature(part),
                CallMode::Tools => None,
            },
        };
        match mode {
            CallMode::LegacyFunction => function = Some(chunk),
            CallMode::Tools => {
                let id = call.id.clone().unwrap_or_else(|| synthetic_id(ids));
                tools.push(tool_chunk(part, id, chunk, index));
            }
        }
        index = index.checked_add(1).ok_or(PartsError::ToolIndex)?;
    }
    Ok((function, (!tools.is_empty()).then_some(tools), index))
}
fn tool_chunk(part: &Part, id: String, function: FunctionChunk, index: u32) -> ToolCallChunk {
    let encoded = match &part.thought_signature {
        Some(signature) => encode_tool_call_id(&id, Some(signature)),
        None => id,
    };
    ToolCallChunk {
        id: encoded,
        kind: PartKind::Function,
        function,
        index,
        provider_specific_fields: signature(part),
    }
}
fn arguments(args: Option<&Json>) -> Result<String, PartsError> {
    let Some(value) = args else {
        return Ok(String::from("{}"));
    };
    serde_json::to_string(value).map_err(|source| PartsError::Arguments { source })
}
fn signature(part: &Part) -> Option<SignatureFields> {
    let signature = part.thought_signature.as_ref()?;
    Some(SignatureFields {
        thought_signature: signature.clone(),
    })
}
fn synthetic_id(ids: &mut dyn FnMut() -> Uuid) -> String {
    let simple = ids().simple().to_string();
    let prefix: String = simple.chars().take(28).collect();
    format!("call_{prefix}")
}
pub(crate) fn invocations(parts: &[Part]) -> Option<Vec<ServerToolInvocation>> {
    let mut calls: IndexMap<String, ServerToolInvocation> = IndexMap::new();
    let mut responses: IndexMap<String, ServerToolInvocation> = IndexMap::new();
    for part in parts {
        match (&part.tool_call, &part.tool_response) {
            (Some(call), _) => {
                calls.insert(call.id.clone().unwrap_or_default(), entry(part, call, true));
            }
            (None, Some(response)) => {
                responses.insert(
                    response.id.clone().unwrap_or_default(),
                    entry(part, response, false),
                );
            }
            (None, None) => {}
        }
    }
    let mut merged = Vec::new();
    for (call_id, mut call) in calls {
        if let Some(response) = responses.shift_remove(&call_id) {
            call.response = response.response;
            call.response_thought_signature = response.thought_signature;
        }
        merged.push(call);
    }
    for (_, mut response) in responses {
        if let Some(signature) = response.thought_signature.clone() {
            response.response_thought_signature = Some(signature);
        }
        merged.push(response);
    }
    (!merged.is_empty()).then_some(merged)
}
fn entry(part: &Part, tool: &ServerTool, is_call: bool) -> ServerToolInvocation {
    ServerToolInvocation {
        tool_type: tool.tool_type.clone(),
        id: tool.id.clone(),
        args: is_call.then(|| tool.args.clone().unwrap_or(Json::Null)),
        thought_signature: part.thought_signature.clone(),
        response: if is_call {
            None
        } else {
            Some(tool.response.clone().unwrap_or(Json::Null))
        },
        response_thought_signature: None,
    }
}
