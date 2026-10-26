pub mod response;
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use response::RawGenericGuardrailAPIResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardrailInputType {
    Request,
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreachableFallback {
    FailClosed,
    FailOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingTransformMode {
    BlockOnly,
    IncrementalDiff,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum GuardrailExtraValue {
    Null,
    Boolean(bool),
    SignedInteger(i64),
    UnsignedInteger(u64),
    Decimal(f64),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GuardrailMessage {
    pub role: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailExtraValue>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GuardrailToolParam {
    #[serde(rename = "type")]
    pub tool_type: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailExtraValue>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GenericGuardrailAPIMetadata {
    pub user_api_key_hash: Option<String>,
    pub user_api_key_alias: Option<String>,
    pub user_api_key_user_id: Option<String>,
    pub user_api_key_user_email: Option<String>,
    pub user_api_key_team_id: Option<String>,
    pub user_api_key_team_alias: Option<String>,
    pub user_api_key_end_user_id: Option<String>,
    pub user_api_key_org_id: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GuardrailOptions {
    #[serde(rename = "additional_provider_specific_params")]
    pub provider_params: Option<BTreeMap<String, GuardrailExtraValue>>,
    pub unreachable_fallback: Option<UnreachableFallback>,
    pub fail_on_error: Option<bool>,
    pub streaming_end_of_stream_only: Option<bool>,
    pub streaming_sampling_rate: Option<u32>,
    pub streaming_transform_mode: Option<StreamingTransformMode>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenericGuardrailAPIConfigModel {
    #[serde(default)]
    pub optional_params: Option<GuardrailOptions>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericGuardrailAPISettings {
    pub unreachable_fallback: UnreachableFallback,
    pub fail_on_error: bool,
    pub streaming_end_of_stream_only: bool,
    pub streaming_sampling_rate: u32,
    pub streaming_transform_mode: StreamingTransformMode,
}

impl GenericGuardrailAPISettings {
    #[must_use]
    pub fn from_optional(optional_params: Option<GuardrailOptions>) -> Self {
        let resolved_params = optional_params.unwrap_or_default();
        Self {
            unreachable_fallback: resolved_params
                .unreachable_fallback
                .unwrap_or(UnreachableFallback::FailClosed),
            fail_on_error: resolved_params.fail_on_error.unwrap_or(true),
            streaming_end_of_stream_only: resolved_params
                .streaming_end_of_stream_only
                .unwrap_or(false),
            streaming_sampling_rate: resolved_params.streaming_sampling_rate.unwrap_or(5),
            streaming_transform_mode: resolved_params
                .streaming_transform_mode
                .unwrap_or(StreamingTransformMode::BlockOnly),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenericGuardrailAPIRequest {
    pub input_type: GuardrailInputType,
    pub litellm_call_id: Option<String>,
    pub litellm_trace_id: Option<String>,
    pub structured_messages: Option<Vec<GuardrailMessage>>,
    pub images: Option<Vec<String>>,
    pub tools: Option<Vec<GuardrailToolParam>>,
    pub texts: Option<Vec<String>>,
    pub request_data: GenericGuardrailAPIMetadata,
    pub request_headers: Option<BTreeMap<String, String>>,
    pub litellm_version: Option<String>,
    #[serde(rename = "additional_provider_specific_params")]
    pub provider_params: Option<BTreeMap<String, GuardrailExtraValue>>,
    pub tool_calls: Option<Vec<GuardrailExtraValue>>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(from = "RawGenericGuardrailAPIResponse")]
pub struct GenericGuardrailAPIResponse {
    pub action: String,
    pub texts: Option<Vec<String>>,
    pub images: Option<Vec<String>>,
    pub tools: Option<Vec<GuardrailToolParam>>,
    pub structured_messages: Option<Vec<GuardrailMessage>>,
    pub blocked_reason: Option<String>,
    pub stream_holdback_chars: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HeaderContext {
    pub headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LiteParamsContext {
    pub metadata: Option<HeaderContext>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InboundRequestContext {
    pub proxy_server_request: Option<HeaderContext>,
    pub headers: Option<BTreeMap<String, String>>,
    pub metadata: Option<HeaderContext>,
    pub litellm_metadata: Option<HeaderContext>,
    pub litellm_params: Option<LiteParamsContext>,
}

#[must_use]
pub fn sanitize_inbound_headers(
    headers: &BTreeMap<String, String>,
    extra_allowlist: Option<&[String]>,
) -> BTreeMap<String, String> {
    let mut sanitized = BTreeMap::new();
    for (name, value) in headers {
        let sanitized_value = if header_value_allowed(name, extra_allowlist) {
            value.clone()
        } else {
            "[present]".to_owned()
        };
        sanitized.insert(name.clone(), sanitized_value);
    }
    sanitized
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
            .any(|allowed_name| allowed_name.eq_ignore_ascii_case(name))
    });
    fixed || generic || extra
}

#[must_use]
pub fn extract_inbound_headers(
    request_data: &InboundRequestContext,
    extra_allowlist: Option<&[String]>,
) -> Option<BTreeMap<String, String>> {
    let locations = [
        request_data
            .proxy_server_request
            .as_ref()
            .and_then(|context| context.headers.as_ref()),
        request_data.headers.as_ref(),
        request_data
            .metadata
            .as_ref()
            .and_then(|context| context.headers.as_ref()),
        request_data
            .litellm_metadata
            .as_ref()
            .and_then(|context| context.headers.as_ref()),
        request_data
            .litellm_params
            .as_ref()
            .and_then(|params| params.metadata.as_ref())
            .and_then(|context| context.headers.as_ref()),
    ];
    locations.into_iter().flatten().find_map(|headers| {
        if headers.is_empty() {
            None
        } else {
            Some(sanitize_inbound_headers(headers, extra_allowlist))
        }
    })
}

#[must_use]
pub fn reconcile_structured_rows(
    original_rows: &[GuardrailMessage],
    shown_rows: &[GuardrailMessage],
    returned_rows: &[GuardrailMessage],
) -> Option<Vec<GuardrailMessage>> {
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
