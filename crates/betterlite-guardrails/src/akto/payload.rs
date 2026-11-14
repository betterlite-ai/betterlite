use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::AktoMirroring;
use super::error::AktoError;

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
        let (response_payload, response_headers) = if include_response {
            let headers =
                BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]);
            (
                double_encode(&build_response_body(inputs, request_data))?,
                serde_json::to_string(&headers).map_err(|source| AktoError::Encode { source })?,
            )
        } else {
            (String::from("{}"), String::from("{}"))
        };
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
                .map(|text| message("user", text))
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
            let mut choice = BTreeMap::new();
            choice.insert("message".to_owned(), message("assistant", text));
            GuardrailValue::Object(choice)
        })
        .collect();
    GuardrailValue::Object(BTreeMap::from([(
        "choices".to_owned(),
        GuardrailValue::Array(choices),
    )]))
}

fn message(role: &str, content: &str) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        ("role".to_owned(), GuardrailValue::Text(role.to_owned())),
        (
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        ),
    ]))
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
    let copied = proxy_headers.iter().filter_map(|(name, value)| {
        let text = value_text(value);
        (!name.is_empty() && !text.is_empty()).then(|| (name.to_lowercase(), text))
    });
    headers.extend(copied);
    headers
}

fn build_tag_metadata(request_data: &BTreeMap<String, GuardrailValue>) -> BTreeMap<String, String> {
    let ids = [
        (
            "user_id",
            metadata_value(request_data, "user_api_key_user_id"),
        ),
        (
            "team_id",
            metadata_value(request_data, "user_api_key_team_id"),
        ),
    ];
    let mut tag = BTreeMap::from([("gen-ai".to_owned(), "Gen AI".to_owned())]);
    tag.extend(
        ids.into_iter()
            .filter_map(|(name, value)| Some((name.to_owned(), value?)))
            .filter(|(_, value)| !value.is_empty()),
    );
    tag
}

fn metadata_value(request_data: &BTreeMap<String, GuardrailValue>, key: &str) -> Option<String> {
    ["litellm_metadata", "metadata"]
        .iter()
        .find_map(|container_key| {
            let Some(GuardrailValue::Object(container)) = request_data.get(*container_key) else {
                return None;
            };
            let value = container
                .get(key)
                .filter(|value| !matches!(value, GuardrailValue::Null))?;
            Some(value_text(value).trim().to_owned())
        })
}

fn extract_request_path(request_data: &BTreeMap<String, GuardrailValue>) -> String {
    let Some(GuardrailValue::Object(metadata)) = request_data.get("metadata") else {
        return String::from("/v1/chat/completions");
    };
    match metadata.get("user_api_key_request_route") {
        Some(GuardrailValue::Text(route)) if !route.is_empty() => route.clone(),
        _ => String::from("/v1/chat/completions"),
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
    let header = |name: &str| match proxy_headers.get(name) {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    };
    let Some(ip) = header("x-forwarded-for").or_else(|| header("x-real-ip")) else {
        return String::new();
    };
    match ip.split_once(',') {
        Some((first, _rest)) => first.trim().to_owned(),
        None => ip,
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
        GuardrailValue::Null | GuardrailValue::Boolean(false) => false,
        GuardrailValue::Boolean(true) => true,
        GuardrailValue::SignedInteger(0) | GuardrailValue::UnsignedInteger(0) => false,
        GuardrailValue::SignedInteger(_) | GuardrailValue::UnsignedInteger(_) => true,
        GuardrailValue::Decimal(number) => number.abs() > 0.0_f64,
        GuardrailValue::Text(text) => !text.is_empty(),
        GuardrailValue::Array(items) => !items.is_empty(),
        GuardrailValue::Object(map) => !map.is_empty(),
    }
}
