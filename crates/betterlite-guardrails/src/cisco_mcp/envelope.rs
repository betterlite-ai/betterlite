use serde::Serialize;
use serde_json::value::RawValue;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::normalize::{self, ResultValue};

#[derive(Debug, Clone, Serialize)]
pub struct RequestPayload {
    pub jsonrpc: &'static str,
    pub id: IdValue,
    pub method: MethodValue,
    pub params: ParamsValue,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponsePayload {
    pub jsonrpc: &'static str,
    pub id: IdValue,
    pub method: MethodValue,
    pub params: ParamsValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Box<RawValue>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum IdValue {
    Given(Box<RawValue>),
    Default(&'static str),
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum MethodValue {
    Given(Box<RawValue>),
    Default(&'static str),
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ParamsValue {
    Given(Box<RawValue>),
    Built {
        name: Box<RawValue>,
        arguments: ArgumentsValue,
    },
    Empty(Fields),
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ArgumentsValue {
    Given(Box<RawValue>),
    Empty(Fields),
}

#[must_use]
pub fn build_request_payload(data: &Fields) -> Option<RequestPayload> {
    if matches!(Json::field(data, "jsonrpc"), Json::Text(version) if version == "2.0") {
        let id = default_id(data, &["id", "litellm_call_id"]);
        let method = first_nonfalsy_raw(data, &["method"])
            .map_or(MethodValue::Default("tools/call"), MethodValue::Given);
        let params = first_nonfalsy_raw(data, &["params"])
            .map_or(ParamsValue::Empty(Fields::new()), ParamsValue::Given);
        return Some(RequestPayload {
            jsonrpc: "2.0",
            id,
            method,
            params,
        });
    }
    let name = first_nonfalsy_raw(data, &["mcp_tool_name", "tool_name", "name"])?;
    let given_arguments =
        present_raw(data, "mcp_arguments").or_else(|| present_raw(data, "arguments"));
    let arguments = match given_arguments {
        Some(raw) if matches!(Json::parse(raw.get()), Json::Object(_)) => {
            ArgumentsValue::Given(raw)
        }
        _ => ArgumentsValue::Empty(Fields::new()),
    };
    Some(RequestPayload {
        jsonrpc: "2.0",
        id: default_id(data, &["litellm_call_id"]),
        method: MethodValue::Default("tools/call"),
        params: ParamsValue::Built { name, arguments },
    })
}

fn default_id(data: &Fields, keys: &[&str]) -> IdValue {
    let raw = first_nonfalsy_raw(data, keys);
    raw.map_or(IdValue::Default("litellm-mcp"), IdValue::Given)
}

#[must_use]
pub fn build_response_payload(request_data: &Fields, response: &Json) -> Option<ResponsePayload> {
    let request_payload = build_request_payload(request_data)?;
    let normalized = normalize::normalize_response(response)?;
    let id = match normalized.id_override {
        Some(raw) => IdValue::Given(raw),
        None => fallback_request_id(request_payload.id, request_data),
    };
    Some(ResponsePayload {
        jsonrpc: request_payload.jsonrpc,
        id,
        method: request_payload.method,
        params: request_payload.params,
        result: normalized.result,
        error: normalized.error,
    })
}

fn fallback_request_id(current: IdValue, request_data: &Fields) -> IdValue {
    if !is_default_id(&current) {
        return current;
    }
    first_nonfalsy_raw(request_data, &["litellm_call_id", "id"]).map_or(current, IdValue::Given)
}

fn is_default_id(id: &IdValue) -> bool {
    match id {
        IdValue::Default(_) => true,
        IdValue::Given(raw) => {
            matches!(Json::parse(raw.get()), Json::Text(value) if value == "litellm-mcp")
        }
    }
}

fn first_nonfalsy_raw(fields: &Fields, keys: &[&str]) -> Option<Box<RawValue>> {
    for key in keys {
        let Some(raw) = fields.get(*key) else {
            continue;
        };
        if Json::parse(raw.get()).truthy() {
            return Some(raw.clone());
        }
    }
    None
}

fn present_raw(fields: &Fields, key: &str) -> Option<Box<RawValue>> {
    let raw = fields.get(key)?;
    (!matches!(Json::parse(raw.get()), Json::Null)).then(|| raw.clone())
}

mod tests;
