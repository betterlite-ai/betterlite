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
    SupportedParams {
        model: String,
    },
    MapParams {
        params: Object,
    },
    Sanitize {
        params: Object,
    },
    Url {
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        global_base: Option<String>,
        #[serde(default)]
        volcengine_base: Option<String>,
        #[serde(default)]
        ark_base: Option<String>,
    },
    Headers {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        global_key: Option<String>,
        #[serde(default)]
        ark_key: Option<String>,
        #[serde(default)]
        volcengine_key: Option<String>,
        #[serde(default)]
        extra_headers: Headers,
    },
    StreamEvent {
        event: Value,
    },
    Reply {
        operation: String,
        status: u16,
        body: String,
        #[serde(default)]
        headers: Headers,
    },
    BodyReply {
        operation: String,
        status: u16,
        body: String,
        #[serde(default)]
        headers: Headers,
    },
    Capabilities {
        #[serde(default)]
        model: Option<String>,
        stream: bool,
    },
}
