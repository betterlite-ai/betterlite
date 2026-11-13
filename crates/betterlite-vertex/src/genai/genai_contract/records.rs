use betterlite_error::facts::presence::Field;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

// LiteLLM's ContentListUnion: one content or a list of contents, in whatever shape Google accepts
pub type ContentListUnion = Json;

// LiteLLM's ToolConfigDict: dict[str, Any] at runtime, and Google takes tools as JSON
pub type ToolConfigDict = Json;

// LiteLLM's GenerateContentConfigDict: the generationConfig mapping, keys kept in the caller's spelling
pub type GenerateContentConfigDict = Object;

// LiteLLM's GenerateContentRequestDict: model, contents, tools and generationConfig, extras kept as sent
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentRequestDict {
    pub model: String,
    pub contents: ContentListUnion,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub tools: Field<ToolConfigDict>,
    #[serde(default, skip_serializing_if = "Field::is_missing")]
    pub generation_config: Field<GenerateContentConfigDict>,
    #[serde(flatten)]
    pub extra: Object,
}

// LiteLLM's GenerateContentResponse: every field the reply carries, plus the metadata kept off the wire
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GenerateContentResponse {
    #[serde(flatten)]
    pub wire: Object,
    // _hidden_params comes in on the constructor payload as pydantic's private attribute and never leaves
    #[serde(rename = "_hidden_params", default, skip_serializing)]
    pub hidden_params: Object,
}
