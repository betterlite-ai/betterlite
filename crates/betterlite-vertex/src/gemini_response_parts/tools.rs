use crate::gemini_response_parts::projection::{CallMode, Part, PartsError, ServerTool};
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
    pub kind: &'static str,
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
        let signed = part
            .thought_signature
            .clone()
            .map(|thought_signature| SignatureFields { thought_signature });
        let chunk = FunctionChunk {
            name: call.name.clone(),
            arguments: arguments(call.args.as_ref())?,
            provider_specific_fields: None,
        };
        match mode {
            CallMode::LegacyFunction => {
                function = Some(FunctionChunk {
                    provider_specific_fields: signed,
                    ..chunk
                });
            }
            CallMode::Tools => {
                let id = call.id.clone().unwrap_or_else(|| {
                    let simple = ids().simple().to_string();
                    format!("call_{}", simple.chars().take(28).collect::<String>())
                });
                tools.push(ToolCallChunk {
                    id: encode_tool_call_id(&id, part.thought_signature.as_deref()),
                    kind: "function",
                    function: chunk,
                    index,
                    provider_specific_fields: signed,
                });
            }
        }
        index = index.checked_add(1).ok_or(PartsError::ToolIndex)?;
    }
    Ok((function, (!tools.is_empty()).then_some(tools), index))
}
fn arguments(args: Option<&Json>) -> Result<String, PartsError> {
    let Some(value) = args else {
        return Ok(String::from("{}"));
    };
    serde_json::to_string(value).map_err(|source| PartsError::Arguments { source })
}
pub(crate) fn invocations(parts: &[Part]) -> Option<Vec<ServerToolInvocation>> {
    let mut calls: IndexMap<String, ServerToolInvocation> = IndexMap::new();
    let mut responses: IndexMap<String, ServerToolInvocation> = IndexMap::new();
    for part in parts {
        if let Some(call) = &part.tool_call {
            calls.insert(call.id.clone().unwrap_or_default(), entry(part, call, true));
            continue;
        }
        if let Some(response) = &part.tool_response {
            let key = response.id.clone().unwrap_or_default();
            responses.insert(key, entry(part, response, false));
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
        let signature = response.thought_signature.clone();
        response.response_thought_signature = signature;
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
        response: (!is_call).then(|| tool.response.clone().unwrap_or(Json::Null)),
        response_thought_signature: None,
    }
}
