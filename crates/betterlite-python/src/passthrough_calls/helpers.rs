use betterlite_config::sections::settings::JsonValue;
use betterlite_context::bedrock_model::runtime_arn::encode_runtime_modelid_arn as encode_arn;
use betterlite_models::passthrough_policies::generic_timeout::{
    DEFAULT_TIMEOUT_SECONDS, resolve_passthrough_timeout as resolve_generic,
};
use betterlite_models::passthrough_policies::header_forwarding::forward_headers_from_request as forward_from;
use betterlite_models::passthrough_policies::query_merge::{
    QueryParameters, merge_query_parameters as merge_query,
};
use betterlite_models::passthrough_policies::sdk_timeout::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeouts, TimeoutValue,
    resolve_sdk_timeout as resolve_sdk,
};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use url::Url;

mod tests;

// BasePassthroughUtils.get_merged_query_parameters: the URL's own query, then the caller's, over the defaults.
#[pyfunction]
pub(crate) fn merge_query_parameters(
    existing_url: &str,
    request_query_params: &str,
    default_query_params: Option<&str>,
) -> PyResult<String> {
    let url = Url::parse(existing_url)
        .map_err(|error| PyValueError::new_err(format!("existing_url is not a URL: {error}")))?;
    let request: QueryParameters = read(request_query_params)?;
    let defaults: Option<QueryParameters> = match default_query_params {
        Some(text) => Some(read(text)?),
        None => None,
    };
    encode(&merge_query(&url, &request, defaults.as_ref()))
}

// BasePassthroughUtils.forward_headers_from_request, with B3571's protected header list.
#[pyfunction]
pub(crate) fn forward_headers_from_request(
    request_headers: &str,
    headers: &str,
    forward_headers: bool,
) -> PyResult<String> {
    let request: IndexMap<String, String> = read(request_headers)?;
    let provided: IndexMap<String, String> = read(headers)?;
    encode(&forward_from(&request, provided, forward_headers))
}

// CommonUtils.encode_bedrock_runtime_modelid_arn, which encodes the resource separator in a model ARN.
#[pyfunction]
pub(crate) fn encode_bedrock_runtime_modelid_arn(endpoint: &str) -> String {
    encode_arn(endpoint)
}

// resolve_pass_through_request_timeout: the endpoint's own timeout, else the 600 second default.
#[pyfunction]
pub(crate) fn resolve_passthrough_timeout(endpoint_timeout: Option<f64>) -> f64 {
    resolve_generic(endpoint_timeout, DEFAULT_TIMEOUT_SECONDS)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SdkTimeoutRequest {
    kwargs: Object,
    litellm_params: Object,
    router_timeout: Option<JsonValue>,
    router_stream_timeout: Option<JsonValue>,
}

// resolve_llm_passthrough_timeout: the stream levels first, then the generic chain, then 600 seconds.
#[pyfunction]
pub(crate) fn resolve_sdk_timeout(request: &str) -> PyResult<f64> {
    let wire: SdkTimeoutRequest = read(request)?;
    let request_timeouts = RequestTimeouts {
        stream: matches!(wire.kwargs.get("stream"), Some(JsonValue::Bool(true))),
        stream_timeout: timeout_value(wire.kwargs.get("stream_timeout")),
        timeout: timeout_value(wire.kwargs.get("timeout")),
        request_timeout: timeout_value(wire.kwargs.get("request_timeout")),
    };
    let deployment = DeploymentTimeouts {
        stream_timeout: timeout_value(wire.litellm_params.get("stream_timeout")),
        timeout: timeout_value(wire.litellm_params.get("timeout")),
        request_timeout: timeout_value(wire.litellm_params.get("request_timeout")),
    };
    let router = RouterTimeouts {
        stream_timeout: timeout_value(wire.router_stream_timeout.as_ref()),
        timeout: timeout_value(wire.router_timeout.as_ref()),
    };
    resolve_sdk(
        &request_timeouts,
        &deployment,
        &router,
        None,
        DEFAULT_TIMEOUT_SECONDS,
    )
    .map_err(|error| PyValueError::new_err(error.to_string()))
}

// only the winning value is validated, so a list or object in a lower field never fails the call
fn timeout_value(value: Option<&JsonValue>) -> Option<TimeoutValue> {
    match value {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::Number(number)) => number.as_f64().map(TimeoutValue::Number),
        Some(JsonValue::Bool(flag)) => {
            Some(TimeoutValue::Number(if *flag { 1.0_f64 } else { 0.0_f64 }))
        }
        Some(JsonValue::String(text)) => Some(TimeoutValue::Text(text.clone())),
        Some(other @ (JsonValue::Array(_) | JsonValue::Object(_))) => {
            Some(TimeoutValue::Text(other.to_string()))
        }
    }
}

fn read<T>(text: &str) -> PyResult<T>
where
    T: for<'text> Deserialize<'text>,
{
    serde_json::from_str(text).map_err(|error| PyValueError::new_err(error.to_string()))
}

fn encode<T>(value: &T) -> PyResult<String>
where
    T: Serialize,
{
    serde_json::to_string(value).map_err(|error| PyRuntimeError::new_err(error.to_string()))
}
