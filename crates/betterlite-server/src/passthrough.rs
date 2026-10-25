#![expect(
    clippy::disallowed_types,
    reason = "pass-through records preserve provider JSON that betterlite does not model"
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

pub const CUSTOM_BODY_STATE_KEY: &str = "litellm_pass_through_custom_body";
pub const RAW_BODY_STATE_KEY: &str = "litellm_pass_through_raw_body";
pub const MODEL_INFO_STATE_KEY: &str = "litellm_pass_through_deployment_model_info";
pub const ENDPOINT_MARKER: &str = "__litellm_pass_through_endpoint__";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointType {
    VertexAi,
    Gemini,
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
    Tinyfish,
    Generic,
    Decisions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PassthroughLogPayload {
    pub url: String,
    pub request_method: Option<String>,
    pub request_body: Option<Value>,
    pub response_body: Option<Value>,
    pub cost_per_request: Option<f64>,
}

mod tests;

#[must_use]
pub fn get_endpoint_type(url: &str, custom_llm_provider: Option<&str>) -> EndpointType {
    let parsed = Url::parse(url).ok();
    let path = parsed.as_ref().map_or("", Url::path);
    let host_name = parsed.as_ref().and_then(Url::host_str).unwrap_or_default();
    if custom_llm_provider == Some("typesafe")
        && path
            .strip_suffix('/')
            .unwrap_or(path)
            .ends_with("/v1/systemone")
    {
        return EndpointType::Decisions;
    }
    if [
        "generateContent",
        "streamGenerateContent",
        "rawPredict",
        "streamRawPredict",
    ]
    .iter()
    .any(|marker| url.contains(marker))
    {
        return EndpointType::VertexAi;
    }
    match host_name {
        "api.anthropic.com" => EndpointType::Anthropic,
        "openai.azure.com" => EndpointType::OpenAi,
        openai_host if openai_host.contains("openai.com") => EndpointType::OpenAi,
        _ if is_tinyfish_agent_url(url) => EndpointType::Tinyfish,
        _ => EndpointType::Generic,
    }
}

#[must_use]
pub fn is_tinyfish_agent_url(url: &str) -> bool {
    let parsed = Url::parse(url).ok();
    let host = parsed.as_ref().and_then(Url::host_str);
    host.is_some_and(|value| value == "agent.tinyfish.ai")
}
