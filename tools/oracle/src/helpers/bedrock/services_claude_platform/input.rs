use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    WorkspaceIdRequired {
        model: String,
        cases: Vec<WorkspaceCase>,
    },
    ChatPlatformHeaders(Box<HeaderInput>),
    NativePlatformHeaders(Box<HeaderInput>),
    ChatPlatformBody(Box<BodyInput>),
    NativePlatformBody(Box<BodyInput>),
    NativeAnthropicStream {
        id: String,
        json_mode: Option<bool>,
        lines: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkspaceCase {
    #[serde(default)]
    pub(crate) optional_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) deployment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) litellm_settings: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Vec<Json>,
    #[serde(default)]
    pub(crate) optional_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) headers: IndexMap<String, String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) deployment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) litellm_settings: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyInput {
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    #[serde(default)]
    pub(crate) optional_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) headers: IndexMap<String, String>,
}
