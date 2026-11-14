use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    SupportedParams(SupportedParams),
    Sanitize(Sanitize),
    Url(Url),
    Headers(HeadersStep),
    StreamEvent(StreamEvent),
    Reply(ReplyStep),
    Capabilities(Capabilities),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupportedParams {
    #[serde(rename = "model")]
    _model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sanitize {
    pub(crate) params: Object,
}

// the four slots hold the same names LiteLLM reads a base from, so each maps straight across
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Url {
    #[serde(default, rename = "api_base")]
    pub(crate) api: Option<String>,
    #[serde(default, rename = "global_base")]
    pub(crate) global: Option<String>,
    #[serde(default, rename = "volcengine_base")]
    pub(crate) volcengine: Option<String>,
    #[serde(default, rename = "ark_base")]
    pub(crate) ark: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeadersStep {
    #[serde(default, rename = "api_key")]
    pub(crate) api: Option<String>,
    #[serde(default, rename = "global_key")]
    pub(crate) global: Option<String>,
    #[serde(default, rename = "ark_key")]
    pub(crate) ark: Option<String>,
    #[serde(default, rename = "volcengine_key")]
    pub(crate) volcengine: Option<String>,
    #[serde(default)]
    pub(crate) extra_headers: Headers,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamEvent {
    pub(crate) event: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyStep {
    pub(crate) operation: String,
    pub(crate) status: u16,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) headers: Headers,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Capabilities {
    #[serde(default, rename = "model")]
    _model: Option<String>,
    #[serde(rename = "stream")]
    _stream: bool,
}
