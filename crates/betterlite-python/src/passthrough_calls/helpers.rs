use super::convert::{ConvertError, encode, parse};
use betterlite_config::sections::settings::JsonValue;
use betterlite_context::bedrock_model::runtime_arn::encode_runtime_modelid_arn as runtime_arn;
use betterlite_models::passthrough_policies::generic_timeout::{
    DEFAULT_TIMEOUT_SECONDS, resolve_passthrough_timeout as resolve_generic,
};
use betterlite_models::passthrough_policies::header_forwarding::forward_headers_from_request as forward_from;
use betterlite_models::passthrough_policies::query_merge::{
    QueryParameters, merge_query_parameters as merge_query,
};
use betterlite_models::passthrough_policies::sdk_timeout::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeouts, TimeoutError,
    resolve_sdk_timeout as resolve_sdk, timeout_value,
};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use pyo3::prelude::*;
use serde::Deserialize;
use thiserror::Error;
use url::{ParseError, Url};

mod tests;

// the base a scheme-less existing_url resolves against; only the query is read
const BASE_URL: &str = "http://localhost/";

#[derive(Debug, Error)]
pub(crate) enum HelperError {
    #[error("existing_url is not a URL: {0}")]
    Url(#[from] ParseError),
    #[error(transparent)]
    Wire(#[from] ConvertError),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
}

// BasePassthroughUtils.get_merged_query_parameters: the URL's own query, then the caller's, over the defaults.
#[pyfunction]
pub(crate) fn merge_query_parameters(
    existing_url: &str,
    request_query_params: &str,
    default_query_params: Option<&str>,
) -> Result<String, HelperError> {
    // LiteLLM reads only the query, so a scheme-less reference merges like an absolute URL
    let url = match Url::parse(existing_url) {
        Ok(url) => url,
        Err(_unparsed) => Url::parse(BASE_URL)?.join(existing_url)?,
    };
    let request: QueryParameters = parse(request_query_params)?;
    let defaults: Option<QueryParameters> = match default_query_params {
        Some(text) => Some(parse(text)?),
        None => None,
    };
    let merged = merge_query(&url, &request, defaults.as_ref());
    let answer = encode(&merged)?;
    Ok(answer)
}

// BasePassthroughUtils.forward_headers_from_request, with B3571's protected header list.
#[pyfunction]
pub(crate) fn forward_headers_from_request(
    request_headers: &str,
    headers: &str,
    forward_headers: bool,
) -> Result<String, HelperError> {
    let request: IndexMap<String, String> = parse(request_headers)?;
    let provided: IndexMap<String, String> = parse(headers)?;
    let forwarded = forward_from(&request, provided, forward_headers);
    let answer = encode(&forwarded)?;
    Ok(answer)
}

// CommonUtils.encode_bedrock_runtime_modelid_arn, which encodes the resource separator in a model ARN.
#[pyfunction]
#[pyo3(name = "encode_bedrock_runtime_modelid_arn")]
// ast-grep-ignore: no-one-line-functions - a pyfunction's only call site is Python, which cannot call the crate function
pub(crate) fn encode_arn(endpoint: &str) -> String {
    runtime_arn(endpoint)
}

// resolve_pass_through_request_timeout: the endpoint's own timeout, else the 600 second default.
#[pyfunction]
// ast-grep-ignore: no-one-line-functions - a pyfunction's only call site is Python, which cannot call the policy
pub(crate) const fn resolve_passthrough_timeout(endpoint_timeout: Option<f64>) -> f64 {
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
pub(crate) fn resolve_sdk_timeout(request: &str) -> Result<f64, HelperError> {
    let wire: SdkTimeoutRequest = parse(request)?;
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
    .map_err(HelperError::from)
}
