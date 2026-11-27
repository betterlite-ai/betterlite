use std::collections::BTreeMap;

use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MappingCall {
    #[serde(default)]
    pub(super) voice: Option<SpeechVoice>,
    #[serde(default)]
    pub(super) optional_params: Object,
    #[serde(default)]
    pub(super) drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Mapping {
    pub(super) model: String,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) calls: Vec<MappingCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestInput {
    pub(super) model: String,
    pub(super) input: String,
    #[serde(default)]
    pub(super) optional_params: Object,
    #[serde(default)]
    pub(super) headers: IndexMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Globals {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DispatchInput {
    pub(super) model: String,
    pub(super) input: String,
    #[serde(default)]
    pub(super) voice: Option<SpeechVoice>,
    #[serde(default)]
    pub(super) optional_params: Object,
    #[serde(default)]
    pub(super) litellm_params: Object,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) timeout_seconds: f64,
    #[serde(default)]
    pub(super) extra_headers: Option<Object>,
    #[serde(default)]
    pub(super) aspeech: bool,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) globals: Globals,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyInput {
    pub(super) status: u16,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SubmissionInput {
    pub(super) status: u16,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResponseInput {
    pub(super) model: String,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) submission: SubmissionInput,
    #[serde(default)]
    pub(super) request_headers: IndexMap<String, String>,
    #[serde(default)]
    pub(super) step_millis: u64,
    pub(super) replies: Vec<ReplyInput>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(super) enum Input {
    Mapping(Mapping),
    Request(RequestInput),
    Dispatch(DispatchInput),
    Response(ResponseInput),
}
