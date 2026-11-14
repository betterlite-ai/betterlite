use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum CustomHeadersError {
    #[error("the key's header fields cannot be read: {0}")]
    KeyFields(#[source] serde_json::Error),
}

// The five UserAPIKeyAuth fields get_custom_headers reads, taken from the key's serde form
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct KeyHeaderView {
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub max_budget: Option<f64>,
    pub spend: Option<f64>,
    pub allowed_model_region: Option<String>,
}

impl KeyHeaderView {
    pub fn from_key(key: &UserAPIKeyAuth) -> Result<Self, CustomHeadersError> {
        serde_json::from_value(serde_json::to_value(key).map_err(CustomHeadersError::KeyFields)?)
            .map_err(CustomHeadersError::KeyFields)
    }
}

// The four attributes get_custom_headers reads off LiteLLM's Logging object
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LoggingHeaderView {
    pub cost_breakdown: Option<JsonMap>,
    pub call_type: String,
    pub litellm_params: Option<JsonMap>,
    pub response_timing_metrics: Option<JsonMap>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CustomHeadersInput {
    pub key: KeyHeaderView,
    pub call_id: Option<String>,
    pub model_id: Option<String>,
    pub cache_key: Option<String>,
    pub api_base: Option<String>,
    pub version: Option<String>,
    pub model_region: Option<String>,
    pub response_cost: Option<JsonValue>,
    pub hidden_params: JsonMap,
    pub fastest_response_batch_completion: Option<bool>,
    pub request_data: JsonMap,
    pub timeout: Option<f64>,
    pub logging: Option<LoggingHeaderView>,
    #[serde(default = "default_read_timing")]
    pub read_timing_from_logging: bool,
    pub extra: Vec<(String, JsonValue)>,
}

// LiteLLM's read_timing_from_logging_obj default: a caller that omits the flag reads the logging object
const fn default_read_timing() -> bool {
    true
}

// LiteLLM's call_id fallback: the given id, then the hidden params' litellm_call_id, then the request data's
pub(super) fn resolved_call_id(input: &CustomHeadersInput) -> Option<String> {
    if let Some(call_id) = input.call_id.as_deref().filter(|text| !text.is_empty()) {
        return Some(call_id.to_owned());
    }
    for candidate in [
        input.hidden_params.get("litellm_call_id"),
        input.request_data.get("litellm_call_id"),
    ] {
        if let Some(value) = candidate.filter(|value| truthy(value)) {
            return python_str("x-litellm-call-id", value);
        }
    }
    None
}

// LiteLLM keeps the query string off the api base, since it can carry a key
pub(super) fn api_base(api_base: Option<&str>) -> Option<String> {
    let base_url = api_base.filter(|text| !text.is_empty())?;
    let base = base_url
        .split_once('?')
        .map_or(base_url, |(base, _query)| base);
    Some(base.to_owned())
}

// LiteLLM's _get_deployment_model_name: the first truthy deployment under litellm_metadata, then metadata
pub(super) fn deployment_model_name(logging: Option<&LoggingHeaderView>) -> Option<String> {
    let params = logging?.litellm_params.as_ref()?;
    for bucket in ["litellm_metadata", "metadata"] {
        let Some(JsonValue::Object(metadata)) = params.get(bucket) else {
            continue;
        };
        if let Some(deployment) = metadata.get("deployment").filter(|value| truthy(value)) {
            return python_str("x-litellm-model-name", deployment);
        }
    }
    None
}

// LiteLLM's _timing_values: both timing values come from one source, so the headers describe one window
pub(super) fn timing_values(
    input: &CustomHeadersInput,
) -> (Option<&JsonValue>, Option<&JsonValue>) {
    let hidden = &input.hidden_params;
    let from_hidden = hidden
        .get("_response_ms")
        .is_some_and(|value| !value.is_null())
        || !input.read_timing_from_logging
        || input.logging.is_none();
    let source = if from_hidden {
        Some(hidden)
    } else {
        input
            .logging
            .as_ref()
            .and_then(|logging| logging.response_timing_metrics.as_ref())
    };
    match source {
        Some(values) => (
            values.get("_response_ms"),
            values.get("litellm_overhead_time_ms"),
        ),
        None => (None, None),
    }
}

// Python's str() of a JSON value; a list or object has no header text and is left out
pub(super) fn python_str(header: &str, value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Null => Some("None".to_owned()),
        JsonValue::Bool(flag) => Some(if *flag { "True" } else { "False" }.to_owned()),
        JsonValue::Number(number) => Some(python_number(number)),
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Array(_) | JsonValue::Object(_) => {
            tracing::warn!(
                header,
                "a response header value must be a scalar, so this one is left out"
            );
            None
        }
    }
}

fn python_number(number: &serde_json::Number) -> String {
    match number.as_f64() {
        Some(float) if number.is_f64() => python_float(float),
        Some(_) | None => number.to_string(),
    }
}

// Python's float repr stays in fixed notation only for decimal exponents -4 through 15
pub(super) fn python_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    if value == 0.0 || (1e-4_f64..1e16_f64).contains(&value.abs()) {
        let fixed = value.to_string();
        let point = if fixed.contains('.') { "" } else { ".0" };
        return format!("{fixed}{point}");
    }
    let scientific = format!("{value:e}");
    match scientific.split_once('e') {
        Some((mantissa, power)) => power.strip_prefix('-').map_or_else(
            || format!("{mantissa}e+{power:0>2}"),
            |digits| format!("{mantissa}e-{digits:0>2}"),
        ),
        None => scientific,
    }
}

pub(super) fn truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number.as_f64().is_none_or(|float| float != 0.0),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}
