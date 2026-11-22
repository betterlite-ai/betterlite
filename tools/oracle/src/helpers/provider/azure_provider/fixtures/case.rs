use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_openai::openai_base_config::Headers;
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;
use serde_json::Value;

const DEFAULT_STATUS: u16 = 200;

const fn default_status() -> u16 {
    DEFAULT_STATUS
}

pub(crate) fn failure<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Chat,
    Embedding,
    Text,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct CaseGlobals {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_version: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) azure_key: Option<String>,
    #[serde(default)]
    pub(crate) headers: Option<Headers>,
    #[serde(default)]
    pub(crate) azure_default_api_version: Option<String>,
}

// One Azure call, keyed the way main.py reads its own arguments; a second call proves per-call resolution.
#[derive(Debug, Deserialize)]
pub(crate) struct Case {
    #[serde(rename = "case")]
    pub(crate) operation: Operation,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_version: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) azure_ad_token: Option<String>,
    #[serde(default)]
    pub(crate) headers: Option<Headers>,
    #[serde(default)]
    pub(crate) extra_headers: Option<Headers>,
    #[serde(default)]
    pub(crate) extra_body: Option<Value>,
    #[serde(default)]
    pub(crate) max_retries: Option<Value>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) globals: CaseGlobals,
    #[serde(default)]
    pub(crate) chat_defaults: Option<Value>,
    #[serde(default)]
    pub(crate) o_series_defaults: Option<Value>,
    #[serde(default)]
    pub(crate) messages: Vec<Value>,
    #[serde(default)]
    pub(crate) optional_params: Option<Value>,
    #[serde(default)]
    pub(crate) embedding: Option<Value>,
    #[serde(default)]
    pub(crate) acompletion: bool,
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) reply: Value,
    #[serde(default)]
    pub(crate) second: Option<Box<Self>>,
}
