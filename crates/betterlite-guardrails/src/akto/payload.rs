use std::cmp::Ordering;
use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use super::AktoMirroring;
use super::error::AktoError;

pub const DEFAULT_ROUTE: &str = "/v1/chat/completions";

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AktoInputs {
    pub texts: Vec<String>,
    pub model: Option<String>,
    pub tools: Option<GuardrailValue>,
    pub tool_calls: Option<GuardrailValue>,
    pub structured_messages: Option<GuardrailValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AktoPayload {
    pub path: String,
    pub request_headers: String,
    pub response_headers: String,
    pub method: String,
    pub request_payload: String,
    pub response_payload: String,
    pub ip: String,
    pub dest_ip: String,
    pub time: String,
    pub status_code: String,
    #[serde(rename = "type")]
    pub request_type: String,
    pub status: String,
    #[serde(rename = "akto_account_id")]
    pub akto_account_id: String,
    #[serde(rename = "akto_vxlan_id")]
    pub akto_vxlan_id: String,
    #[serde(rename = "is_pending")]
    pub is_pending: String,
    pub source: String,
    #[serde(rename = "process_id")]
    pub process_id: Option<String>,
    #[serde(rename = "socket_id")]
    pub socket_id: Option<String>,
    #[serde(rename = "daemonset_id")]
    pub daemonset_id: Option<String>,
    #[serde(rename = "enabled_graph")]
    pub enabled_graph: Option<String>,
    pub tag: String,
    pub metadata: String,
    pub context_source: String,
}

impl AktoMirroring {
    pub fn build_akto_payload(
        &self,
        inputs: &AktoInputs,
        request_data: &BTreeMap<String, GuardrailValue>,
        status_code: u16,
        include_response: bool,
    ) -> Result<AktoPayload, AktoError> {
        let request_body = build_request_body(inputs, request_data);
        let request_headers = build_request_headers(request_data);
        let tag = build_tag_metadata(request_data);
        let mut response_payload = serde_json::to_string(&BTreeMap::<String, String>::new())
            .map_err(|source| AktoError::Encode { source })?;
        let mut response_headers = response_payload.clone();
        if include_response {
            response_payload = double_encode(&build_response_body(inputs, request_data))?;
            response_headers = serde_json::to_string(&BTreeMap::from([(
                "content-type".to_owned(),
                "application/json".to_owned(),
            )]))
            .map_err(|source| AktoError::Encode { source })?;
        }
        let tag_text =
            serde_json::to_string(&tag).map_err(|source| AktoError::Encode { source })?;
        let status = status_code.to_string();
        Ok(AktoPayload {
            path: extract_request_path(request_data),
            request_headers: serde_json::to_string(&request_headers)
                .map_err(|source| AktoError::Encode { source })?,
            response_headers,
            method: "POST".to_owned(),
            request_payload: double_encode(&request_body)?,
            response_payload,
            ip: extract_client_ip(request_data),
            dest_ip: "127.0.0.1".to_owned(),
            time: (self.now_millis)().to_string(),
            status_code: status.clone(),
            request_type: "HTTP/1.1".to_owned(),
            status,
            akto_account_id: self.account_id.clone(),
            akto_vxlan_id: self.vxlan_id.clone(),
            is_pending: "false".to_owned(),
            source: "MIRRORING".to_owned(),
            direction: None,
            process_id: None,
            socket_id: None,
            daemonset_id: None,
            enabled_graph: None,
            tag: tag_text.clone(),
            metadata: tag_text,
            context_source: "AGENTIC".to_owned(),
        })
    }
}

fn double_encode<T>(body: &T) -> Result<String, AktoError>
where
    T: Serialize,
{
    let inner = serde_json::to_string(body).map_err(|source| AktoError::Encode { source })?;
    serde_json::to_string(&BTreeMap::from([("body", inner)]))
        .map_err(|source| AktoError::Encode { source })
}

fn build_request_body(
    inputs: &AktoInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> GuardrailValue {
    let mut model = inputs.model.clone().unwrap_or_default();
    let mut messages = GuardrailValue::Array(Vec::new());
    if let Some(structured) = inputs
        .structured_messages
        .as_ref()
        .filter(|value| present(value))
    {
        messages = structured.clone();
    } else if let Some(found) = request_data.get("messages").filter(|value| present(value)) {
        messages = found.clone();
        if let Some(GuardrailValue::Text(found_model)) = request_data.get("model") {
            model.clone_from(found_model);
        }
    } else if !inputs.texts.is_empty() {
        messages = GuardrailValue::Array(
            inputs
                .texts
                .iter()
                .map(|text| {
                    let mut message = BTreeMap::new();
                    message.insert("role".to_owned(), GuardrailValue::Text("user".to_owned()));
                    message.insert("content".to_owned(), GuardrailValue::Text(text.to_owned()));
                    GuardrailValue::Object(message)
                })
                .collect(),
        );
    }
    let mut body = BTreeMap::from([
        ("model".to_owned(), GuardrailValue::Text(model)),
        ("messages".to_owned(), messages),
    ]);
    if let Some(tools) = inputs.tools.as_ref().filter(|value| present(value)) {
        body.insert("tools".to_owned(), tools.clone());
    } else if let Some(tools) = request_data.get("tools").filter(|value| present(value)) {
        body.insert("tools".to_owned(), tools.clone());
    }
    if let Some(tool_calls) = inputs.tool_calls.as_ref().filter(|value| present(value)) {
        body.insert("tool_calls".to_owned(), tool_calls.clone());
    }
    GuardrailValue::Object(body)
}

fn build_response_body(
    inputs: &AktoInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> GuardrailValue {
    if let Some(response @ GuardrailValue::Object(_)) = request_data.get("response") {
        return response.clone();
    }
    if inputs.texts.is_empty() {
        return GuardrailValue::Object(BTreeMap::new());
    }
    let choices = inputs
        .texts
        .iter()
        .map(|text| {
            let mut message = BTreeMap::new();
            message.insert("content".to_owned(), GuardrailValue::Text(text.to_owned()));
            message.insert(
                "role".to_owned(),
                GuardrailValue::Text("assistant".to_owned()),
            );
            let mut choice = BTreeMap::new();
            choice.insert("message".to_owned(), GuardrailValue::Object(message));
            GuardrailValue::Object(choice)
        })
        .collect();
    GuardrailValue::Object(BTreeMap::from([(
        "choices".to_owned(),
        GuardrailValue::Array(choices),
    )]))
}

fn build_request_headers(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]);
    let Some(GuardrailValue::Object(proxy_request)) = request_data.get("proxy_server_request")
    else {
        return headers;
    };
    let Some(GuardrailValue::Object(proxy_headers)) = proxy_request.get("headers") else {
        return headers;
    };
    for (name, value) in proxy_headers {
        let text = value_text(value);
        if !name.is_empty() && !text.is_empty() {
            headers.insert(name.to_lowercase(), text);
        }
    }
    headers
}

fn build_tag_metadata(request_data: &BTreeMap<String, GuardrailValue>) -> BTreeMap<String, String> {
    let mut tag = BTreeMap::from([("gen-ai".to_owned(), "Gen AI".to_owned())]);
    if let Some(user_id) = resolve_metadata_value(request_data, "user_api_key_user_id") {
        if !user_id.is_empty() {
            tag.insert("user_id".to_owned(), user_id);
        }
    }
    if let Some(team_id) = resolve_metadata_value(request_data, "user_api_key_team_id") {
        if !team_id.is_empty() {
            tag.insert("team_id".to_owned(), team_id);
        }
    }
    tag
}

fn resolve_metadata_value(
    request_data: &BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Option<String> {
    for container_key in ["litellm_metadata", "metadata"] {
        let Some(GuardrailValue::Object(container)) = request_data.get(container_key) else {
            continue;
        };
        if container.is_empty() {
            continue;
        }
        let Some(value) = container.get(key) else {
            continue;
        };
        if matches!(value, GuardrailValue::Null) {
            continue;
        }
        return Some(value_text(value).trim().to_owned());
    }
    None
}

fn extract_request_path(request_data: &BTreeMap<String, GuardrailValue>) -> String {
    let Some(GuardrailValue::Object(metadata)) = request_data.get("metadata") else {
        return DEFAULT_ROUTE.to_owned();
    };
    match metadata.get("user_api_key_request_route") {
        Some(GuardrailValue::Text(route)) if !route.is_empty() => route.clone(),
        _ => DEFAULT_ROUTE.to_owned(),
    }
}

fn extract_client_ip(request_data: &BTreeMap<String, GuardrailValue>) -> String {
    let Some(GuardrailValue::Object(proxy_request)) = request_data.get("proxy_server_request")
    else {
        return String::new();
    };
    let Some(GuardrailValue::Object(proxy_headers)) = proxy_request.get("headers") else {
        return String::new();
    };
    let Some(ip) = header_text(proxy_headers, "x-forwarded-for")
        .or_else(|| header_text(proxy_headers, "x-real-ip"))
    else {
        return String::new();
    };
    if ip.contains(',') {
        return ip.split(',').next().unwrap_or_default().trim().to_owned();
    }
    ip
}

fn header_text(headers: &BTreeMap<String, GuardrailValue>, name: &str) -> Option<String> {
    match headers.get(name) {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn value_text(value: &GuardrailValue) -> String {
    match value {
        GuardrailValue::Null => String::new(),
        GuardrailValue::Boolean(flag) => flag.to_string(),
        GuardrailValue::SignedInteger(number) => number.to_string(),
        GuardrailValue::UnsignedInteger(number) => number.to_string(),
        GuardrailValue::Decimal(number) => number.to_string(),
        GuardrailValue::Text(text) => text.clone(),
        GuardrailValue::Array(_) | GuardrailValue::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

// LiteLLM selects a payload field with Python truthiness, so an empty list stays out of the body
fn present(value: &GuardrailValue) -> bool {
    match value {
        GuardrailValue::Null => false,
        GuardrailValue::Boolean(flag) => *flag,
        GuardrailValue::SignedInteger(number) => *number != 0,
        GuardrailValue::UnsignedInteger(number) => *number != 0,
        GuardrailValue::Decimal(number) => number
            .partial_cmp(&0.0_f64)
            .is_some_and(|ordering| ordering != Ordering::Equal),
        GuardrailValue::Text(text) => !text.is_empty(),
        GuardrailValue::Array(items) => !items.is_empty(),
        GuardrailValue::Object(map) => !map.is_empty(),
    }
}
