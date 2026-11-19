use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Sdk {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
}

#[derive(Deserialize)]
pub(crate) struct ProbeCase {
    pub(super) model: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct RequestCase {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    pub(super) options: Object,
    pub(super) stored: Option<String>,
}
