mod tests;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardrailInputType {
    Request,
    Response,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GuardrailToolParam {
    #[serde(rename = "type")]
    pub tool_type: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GenericGuardrailAPIMetadata {
    #[serde(flatten)]
    pub fields: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GenericGuardrailAPIOptionalParams {
    pub additional_provider_specific_params: Option<BTreeMap<String, Value>>,
    pub unreachable_fallback: Option<String>,
    pub fail_on_error: Option<bool>,
    pub streaming_end_of_stream_only: Option<bool>,
    pub streaming_sampling_rate: Option<u32>,
    pub streaming_transform_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenericGuardrailAPIConfigModel {
    #[serde(default = "default_optional_params")]
    pub optional_params: Option<GenericGuardrailAPIOptionalParams>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericGuardrailAPISettings {
    pub unreachable_fallback: String,
    pub fail_on_error: bool,
    pub streaming_end_of_stream_only: bool,
    pub streaming_sampling_rate: u32,
    pub streaming_transform_mode: String,
}

impl GenericGuardrailAPISettings {
    pub fn from_optional(optional: Option<GenericGuardrailAPIOptionalParams>) -> Self {
        let optional = optional.unwrap_or_default();
        Self {
            unreachable_fallback: optional
                .unreachable_fallback
                .unwrap_or_else(|| "fail_closed".to_owned()),
            fail_on_error: optional.fail_on_error.unwrap_or(true),
            streaming_end_of_stream_only: optional.streaming_end_of_stream_only.unwrap_or(false),
            streaming_sampling_rate: optional.streaming_sampling_rate.unwrap_or(5),
            streaming_transform_mode: optional
                .streaming_transform_mode
                .unwrap_or_else(|| "block_only".to_owned()),
        }
    }
}

fn default_optional_params() -> Option<GenericGuardrailAPIOptionalParams> {
    let params = GenericGuardrailAPIOptionalParams::default();
    Some(params)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenericGuardrailAPIRequest {
    pub input_type: GuardrailInputType,
    pub litellm_call_id: Option<String>,
    pub litellm_trace_id: Option<String>,
    pub structured_messages: Option<Vec<Value>>,
    pub images: Option<Vec<String>>,
    pub tools: Option<Vec<GuardrailToolParam>>,
    pub texts: Option<Vec<String>>,
    pub request_data: GenericGuardrailAPIMetadata,
    pub request_headers: Option<BTreeMap<String, String>>,
    pub litellm_version: Option<String>,
    pub additional_provider_specific_params: Option<BTreeMap<String, Value>>,
    pub tool_calls: Option<Vec<Value>>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenericGuardrailAPIResponse {
    #[serde(default = "default_action")]
    pub action: String,
    pub texts: Option<Vec<String>>,
    pub images: Option<Vec<String>>,
    pub tools: Option<Vec<GuardrailToolParam>>,
    pub structured_messages: Option<Vec<Value>>,
    pub blocked_reason: Option<String>,
    pub stream_holdback_chars: Option<Vec<u32>>,
}

impl GenericGuardrailAPIResponse {
    pub fn from_value(mut value: Value) -> Result<Self, serde_json::Error> {
        if let Some(holdback) = value.get_mut("stream_holdback_chars") {
            if let Value::Array(values) = holdback {
                let coerced = values.iter().map(coerce_stream_holdback_value).collect();
                *holdback = Value::Array(coerced);
            }
        }
        serde_json::from_value(value)
    }
}

fn default_action() -> String {
    let action = "NONE".to_owned();
    action
}

pub fn coerce_stream_holdback_value(value: &Value) -> u32 {
    match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0),
        Value::String(value) => value.parse::<u32>().unwrap_or(0),
        _ => 0,
    }
}

pub fn sanitize_inbound_headers(
    headers: &BTreeMap<String, String>,
    extra_allowlist: Option<&[String]>,
) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let sanitized_value = if header_value_allowed(name, extra_allowlist) {
                value.clone()
            } else {
                "[present]".to_owned()
            };
            (name.clone(), sanitized_value)
        })
        .collect()
}

fn header_value_allowed(name: &str, extra_allowlist: Option<&[String]>) -> bool {
    let lower_name = name.to_ascii_lowercase();
    let fixed = matches!(
        lower_name.as_str(),
        "host"
            | "accept-encoding"
            | "connection"
            | "accept"
            | "content-type"
            | "user-agent"
            | "content-length"
    );
    let generic = lower_name.starts_with("x-stainless-") || lower_name.starts_with("x-litellm-");
    let extra = extra_allowlist.is_some_and(|names| {
        names
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(name))
    });
    fixed || generic || extra
}

pub fn extract_inbound_headers(
    request_data: &Value,
    extra_allowlist: Option<&[String]>,
) -> Option<BTreeMap<String, String>> {
    let locations = [
        request_data.pointer("/proxy_server_request/headers"),
        request_data.pointer("/headers"),
        request_data.pointer("/metadata/headers"),
        request_data.pointer("/litellm_metadata/headers"),
        request_data.pointer("/litellm_params/metadata/headers"),
    ];
    locations.into_iter().flatten().find_map(|value| {
        let headers = string_map(value)?;
        if headers.is_empty() {
            None
        } else {
            Some(sanitize_inbound_headers(&headers, extra_allowlist))
        }
    })
}

fn string_map(value: &Value) -> Option<BTreeMap<String, String>> {
    let object = value.as_object()?;
    Some(
        object
            .iter()
            .filter_map(|(name, value)| value.as_str().map(|text| (name.clone(), text.to_owned())))
            .collect(),
    )
}

pub fn reconcile_structured_rows(
    original_rows: &[Value],
    shown_rows: &[Value],
    returned_rows: &[Value],
) -> Option<Vec<Value>> {
    if returned_rows.len() != original_rows.len() || shown_rows.len() != original_rows.len() {
        return Some(returned_rows.to_vec());
    }
    if returned_rows
        .iter()
        .zip(shown_rows)
        .all(|(returned, shown)| returned == shown)
    {
        return None;
    }
    Some(
        original_rows
            .iter()
            .zip(shown_rows)
            .zip(returned_rows)
            .map(|((original, shown), returned)| {
                if returned == shown {
                    original.clone()
                } else {
                    returned.clone()
                }
            })
            .collect(),
    )
}
