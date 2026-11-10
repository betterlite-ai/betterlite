use betterlite_azure::azure_chat_base_policy::AzureChatSettings;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    ApiVersionResponseFormat(VersionCall),
    MaxCompletionTokens(ModelCall),
    UnsupportedParam(ModelCall),
    AzureToolChoiceDateGate(MappingCall),
    AzureRequiredToolChoice(MappingCall),
    AzureParameterShapes(MappingCall),
    AzureMessageContent(TransformCall),
    SanitizedToolSchema(SanitizeCall),
    AzureToolChoiceWithoutTools(TransformCall),
    AzureClassDefaults(ClassCall),
    AzureSpecialAuthRegions(SpecialAuthCall),
    AzureErrorFactory(ErrorFactoryCall),
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Settings {
    pub(crate) drop_params: Option<bool>,
    pub(crate) openai_system_messages_first: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Loaded {
    pub(crate) drop_params: bool,
    pub(crate) openai_system_messages_first: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingCall {
    #[serde(default)]
    pub(crate) settings: Settings,
    #[serde(default)]
    pub(crate) steps: Vec<MapStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_version: String,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) global_drop_params: Option<bool>,
    #[serde(default)]
    pub(crate) non_default: Object,
    #[serde(default)]
    pub(crate) optional: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VersionCall {
    #[serde(default)]
    pub(crate) settings: Settings,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) versions: Vec<VersionPair>,
    #[serde(default)]
    pub(crate) steps: Vec<MapStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VersionPair {
    pub(crate) year: String,
    pub(crate) month: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelCall {
    #[serde(default)]
    pub(crate) settings: Settings,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) steps: Vec<MapStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformCall {
    #[serde(default)]
    pub(crate) settings: Settings,
    #[serde(default)]
    pub(crate) steps: Vec<TransformStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformStep {
    #[serde(default)]
    pub(crate) messages: Vec<Message>,
    #[serde(default)]
    pub(crate) optional: Object,
    #[serde(default)]
    pub(crate) openai_system_messages_first: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SanitizeCall {
    #[serde(default)]
    pub(crate) optional: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassCall {
    #[serde(default)]
    pub(crate) steps: Vec<ClassStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassStep {
    #[serde(default)]
    pub(crate) values: AzureChatSettings,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpecialAuthCall {
    #[serde(default)]
    pub(crate) steps: Vec<AuthStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthStep {
    #[serde(default)]
    pub(crate) non_default: Object,
    #[serde(default)]
    pub(crate) optional: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorFactoryCall {
    #[serde(default)]
    pub(crate) steps: Vec<ErrorStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorStep {
    pub(crate) message: String,
    pub(crate) status_code: u16,
    #[serde(default)]
    pub(crate) headers: Option<IndexMap<String, String>>,
}
