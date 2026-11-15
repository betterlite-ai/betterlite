use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Object as ParamsObject;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Gpt5AliasCatalog(AliasCall),
    NativeStream(StreamCall),
    Gpt5Allowlist(AllowlistCall),
    OSeriesAllowlist(AllowlistCall),
    ParameterMapping(MappingCall),
    RequestTransform(TransformCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Install {
    pub(crate) model_cost: BTreeMap<String, Value>,
    #[serde(default)]
    pub(crate) chat_models: Vec<String>,
    #[serde(default)]
    pub(crate) text_models: Vec<String>,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) openai_system_messages_first: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AliasCall {
    pub(crate) install: Install,
    #[serde(default)]
    pub(crate) steps: Vec<AliasStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AliasStep {
    pub(crate) model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamCall {
    pub(crate) install: Install,
    #[serde(default)]
    pub(crate) steps: Vec<StreamStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamStep {
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) stream: Option<bool>,
    #[serde(default)]
    pub(crate) provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AllowlistCall {
    pub(crate) install: Install,
    #[serde(default)]
    pub(crate) models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingCall {
    pub(crate) install: Install,
    #[serde(default)]
    pub(crate) steps: Vec<MapStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) non_default: ParamsObject,
    #[serde(default)]
    pub(crate) optional: ParamsObject,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformCall {
    pub(crate) install: Install,
    #[serde(default)]
    pub(crate) steps: Vec<TransformStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Vec<Message>,
    #[serde(default)]
    pub(crate) tools: Option<Value>,
    #[serde(default)]
    pub(crate) tool_choice: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_effort: Option<String>,
}
