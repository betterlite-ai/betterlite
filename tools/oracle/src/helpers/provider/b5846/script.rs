use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use serde::Deserialize;

use crate::helpers::HelperError;

pub(super) const FIXED_NOW: i64 = 1_700_000_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCase {
    #[serde(default)]
    pub(crate) non_default: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayloadRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    #[serde(default)]
    pub(crate) options: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlRequest {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) tenant: Option<String>,
    #[serde(default)]
    pub(crate) stream: Option<bool>,
    #[serde(default)]
    pub(crate) options: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderRequest {
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyRequest {
    pub(crate) body: String,
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) options: Object,
    #[serde(default)]
    pub(crate) parameters: Object,
    #[serde(default)]
    pub(crate) messages: Vec<Message>,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) now: Option<i64>,
    #[serde(default)]
    pub(crate) counter_fails: bool,
    #[serde(default)]
    pub(crate) encoding_fails: bool,
}

pub(super) const fn default_max_new_tokens() -> i64 {
    4096
}

const fn default_status() -> u16 {
    200
}

pub(super) fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: String::from("Runtime"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
}
