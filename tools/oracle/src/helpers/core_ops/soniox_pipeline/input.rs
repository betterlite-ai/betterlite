use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) pipeline: Case,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Case {
    pub(super) model: String,
    pub(super) timeout_millis: u64,
    pub(super) api_key: String,
    pub(super) api_base: String,
    pub(super) optional_params: Object,
    pub(super) answers: Vec<Answer>,
    pub(super) audio: Option<Audio>,
    pub(super) headers: Option<Headers>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) body: String,
    #[serde(default)]
    pub(super) headers: Headers,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Audio {
    pub(super) filename: String,
    pub(super) content: String,
}
