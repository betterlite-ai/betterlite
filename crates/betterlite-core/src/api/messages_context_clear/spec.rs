use betterlite_models::decisions::Json;
use serde::Deserialize;

// LiteLLM reads trigger and keep as total=False dicts, so a null type or value means the default.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClearCountSpec {
    #[serde(rename = "type", default)]
    pub kind: Option<Json>,
    #[serde(default)]
    pub value: Option<Json>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClearToolUsesEditSpec {
    #[serde(rename = "type", default)]
    pub edit_type: Option<Json>,
    #[serde(default)]
    pub trigger: Option<ClearCountSpec>,
    #[serde(default)]
    pub keep: Option<ClearCountSpec>,
    // a key present with null still counts as present, and LiteLLM warns about it
    #[serde(default, with = "serde_with::rust::double_option")]
    pub clear_at_least: Option<Option<Json>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub exclude_tools: Option<Option<Json>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub clear_tool_inputs: Option<Option<Json>>,
}
