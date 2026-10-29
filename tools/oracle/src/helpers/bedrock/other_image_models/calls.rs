use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct TitanCall {
    pub(crate) non_default: Map<String, Value>,
    pub(crate) optional_params: Map<String, Value>,
    pub(crate) text: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Stability1Call {
    pub(crate) non_default: Map<String, Value>,
    pub(crate) optional_params: Map<String, Value>,
    pub(crate) text: String,
    pub(crate) settings: SettingsCall,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct SettingsCall {
    pub(crate) cfg_scale: Option<Value>,
    pub(crate) seed: Option<Value>,
    pub(crate) steps: Option<Value>,
    pub(crate) width: Option<Value>,
    pub(crate) height: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct TextParamsCall {
    pub(crate) text: String,
    pub(crate) optional_params: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FamilyName {
    Titan,
    Stability1,
    Stability3,
    Nova,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct SizeQualityCall {
    pub(crate) family: Option<FamilyName>,
    pub(crate) non_default: Map<String, Value>,
    pub(crate) optional_params: Map<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct ArtifactsCall {
    pub(crate) family: Option<FamilyName>,
    pub(crate) response_body: Map<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct ProviderErrorCall {
    pub(crate) family: Option<FamilyName>,
    pub(crate) response_body: Option<Map<String, Value>>,
    pub(crate) optional_params: Map<String, Value>,
    pub(crate) text: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct SelectionCall {
    pub(crate) model: Option<String>,
}
