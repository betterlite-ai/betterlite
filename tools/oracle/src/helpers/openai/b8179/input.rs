use betterlite_openai::chat::ChatMessage;
use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    SupportedParams,
    MapParams {
        params: Object,
        #[serde(default)]
        optional_params: Object,
    },
    Headers {
        #[serde(default)]
        headers: Headers,
        api_key: Option<String>,
    },
    ErrorClass {
        message: String,
        status: u16,
        headers: Option<Headers>,
    },
    Defaults {
        defaults: Object,
    },
    Completion(CompletionStep),
    Reply {
        status: u16,
        body: String,
        model: String,
        created: u64,
    },
    StreamLines {
        lines: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionStep {
    pub(crate) model: String,
    pub(crate) api_base: String,
    pub(crate) messages: Vec<ChatMessage>,
    #[serde(default)]
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) defaults: Object,
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
    pub(crate) created: u64,
    pub(crate) reply: Option<ReplyStep>,
    pub(crate) chunks: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyStep {
    pub(crate) status: u16,
    pub(crate) body: String,
}
