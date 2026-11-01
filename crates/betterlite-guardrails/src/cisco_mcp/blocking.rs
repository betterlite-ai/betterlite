use rust_decimal::Decimal;
use serde::Serialize;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;

#[derive(Debug, Clone, Serialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct HiddenParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_cost: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockingResponse {
    pub mcp_tool_call_response: [TextContent; 1],
    pub hidden_params: HiddenParams,
}

#[derive(Debug, Serialize)]
struct SynthesizedPayload<'provider> {
    error: &'static str,
    message: String,
    provider: &'provider str,
    guardrail: Option<&'provider str>,
    surface: &'static str,
    direction: &'static str,
    action: &'static str,
}

#[must_use]
pub fn build_blocking_response(
    detail: &Json,
    provider_name: &str,
    guardrail_name: Option<&str>,
    response_cost: Option<Decimal>,
) -> BlockingResponse {
    let text = match detail {
        Json::Object(fields) => serde_json::to_string(fields).unwrap_or_default(),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            let message = if other.truthy() {
                other.python_str()
            } else {
                "Blocked by Cisco AI Defense Guardrail".to_owned()
            };
            let payload = SynthesizedPayload {
                error: "Blocked by Cisco AI Defense Guardrail",
                message,
                provider: provider_name,
                guardrail: guardrail_name,
                surface: "mcp",
                direction: "output",
                action: "block",
            };
            serde_json::to_string(&payload).unwrap_or_default()
        }
    };
    BlockingResponse {
        mcp_tool_call_response: [TextContent { kind: "text", text }],
        hidden_params: HiddenParams { response_cost },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StructuredMirror {
    pub result: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResultReplacement {
    #[serde(flatten)]
    pub rest: Fields,
    pub content: Vec<TextContent>,
    #[serde(rename = "structuredContent")]
    pub structured_content: Option<StructuredMirror>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ReplacedResponse {
    ToolCallResponse {
        #[serde(flatten)]
        rest: Fields,
        mcp_tool_call_response: Box<Self>,
    },
    Content {
        #[serde(flatten)]
        rest: Fields,
        content: Vec<TextContent>,
        #[serde(rename = "structuredContent")]
        structured_content: Option<StructuredMirror>,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    Result {
        #[serde(flatten)]
        rest: Fields,
        result: ResultReplacement,
    },
    Replacement(Vec<TextContent>),
}

const CONTENT_MIRROR_KEYS: [&str; 5] = [
    "content",
    "structuredContent",
    "structured_content",
    "isError",
    "is_error",
];

#[must_use]
pub fn replace_tool_response(
    response: &Json,
    replacement: &[TextContent],
) -> Option<ReplacedResponse> {
    match response {
        Json::Object(fields) => Some(replace_in_object(fields, replacement)),
        Json::List(_) => Some(ReplacedResponse::Replacement(replacement.to_vec())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => None,
    }
}

fn replace_in_object(fields: &Fields, replacement: &[TextContent]) -> ReplacedResponse {
    if let Some(wrapped) = replace_in_wrapper(fields, replacement) {
        return wrapped;
    }
    if matches!(Json::field(fields, "content"), Json::List(_)) {
        return replace_in_content_list(fields, replacement);
    }
    replace_in_result(fields, replacement)
}

fn replace_in_wrapper(fields: &Fields, replacement: &[TextContent]) -> Option<ReplacedResponse> {
    let raw = fields.get("mcp_tool_call_response")?;
    let inner = Json::parse(raw.get());
    if matches!(inner, Json::Null) {
        return None;
    }
    let replaced_inner = replace_tool_response(&inner, replacement)
        .unwrap_or_else(|| ReplacedResponse::Replacement(replacement.to_vec()));
    let mut rest = fields.clone();
    rest.shift_remove("mcp_tool_call_response");
    Some(ReplacedResponse::ToolCallResponse {
        rest,
        mcp_tool_call_response: Box::new(replaced_inner),
    })
}

fn replace_in_content_list(fields: &Fields, replacement: &[TextContent]) -> ReplacedResponse {
    let mut rest = fields.clone();
    for key in CONTENT_MIRROR_KEYS {
        rest.shift_remove(key);
    }
    ReplacedResponse::Content {
        rest,
        content: replacement.to_vec(),
        structured_content: replacement_structured_content(replacement),
        is_error: true,
    }
}

fn replace_in_result(fields: &Fields, replacement: &[TextContent]) -> ReplacedResponse {
    let mut rest = fields.clone();
    let result_rest = match Json::field(fields, "result") {
        Json::Object(existing) => {
            let mut result_fields = existing;
            for key in CONTENT_MIRROR_KEYS {
                result_fields.shift_remove(key);
            }
            result_fields
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Fields::new()
        }
    };
    rest.shift_remove("result");
    ReplacedResponse::Result {
        rest,
        result: ResultReplacement {
            rest: result_rest,
            content: replacement.to_vec(),
            structured_content: replacement_structured_content(replacement),
            is_error: true,
        },
    }
}

// the real MCP SDK's CallToolResult model only exposes camelCase fields, so mirror into structuredContent.result
#[must_use]
pub fn replacement_structured_content(replacement: &[TextContent]) -> Option<StructuredMirror> {
    let item = replacement.first()?;
    Some(StructuredMirror {
        result: item.text.clone(),
    })
}

#[must_use]
pub fn extract_tool_call_response(response: &Json) -> Json {
    let Json::Object(fields) = response else {
        return response.clone();
    };
    match fields.get("mcp_tool_call_response") {
        Some(raw) if !matches!(Json::parse(raw.get()), Json::Null) => Json::parse(raw.get()),
        _ => response.clone(),
    }
}

mod tests;
