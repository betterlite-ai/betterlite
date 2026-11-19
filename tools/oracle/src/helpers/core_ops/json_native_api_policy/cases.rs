use std::collections::BTreeMap;

use betterlite_models::json_provider_registry::ProviderConfig;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    GenericMessagesUrl,
    JsonMessagesUrl,
    GenericMessagesHeaders,
    JsonMessagesHeaders,
    GenericMessagesTransform,
    JsonMessagesTransform,
    ResponsesUrl,
    ResponsesHeaders,
    ResponsesTransform,
    ResponsesCache,
    GenericResponsesUrl,
    GenericResponsesHeaders,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseInput {
    pub(super) calls: Vec<Call>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub(super) kind: Kind,
    pub(super) model: String,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<SecretString>,
    #[serde(default)]
    pub(super) global_api_key: Option<SecretString>,
    #[serde(default)]
    pub(super) descriptor: Option<ProviderConfig>,
    #[serde(default)]
    pub(super) headers: IndexMap<String, String>,
    #[serde(default)]
    pub(super) optional_params: Object,
    #[serde(default)]
    pub(super) messages: Vec<Json>,
    #[serde(default)]
    pub(super) input: String,
    #[serde(default)]
    pub(super) params: Object,
    #[serde(default)]
    pub(super) first: Option<ProviderConfig>,
    #[serde(default)]
    pub(super) second: Option<ProviderConfig>,
}
