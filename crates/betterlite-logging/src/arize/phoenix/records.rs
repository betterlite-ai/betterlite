use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;
use serde_with::{DefaultOnNull, serde_as};

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct ContentPart {
    #[serde(rename = "type", default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub kind: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub text: String,
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TemplateMessage {
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub content: Vec<ContentPart>,
}

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct TemplateBody {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub messages: Vec<TemplateMessage>,
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PromptVersionData {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub template: TemplateBody,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub invocation_parameters: IndexMap<String, Json>,
    pub model_name: Option<String>,
    pub model_provider: Option<String>,
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnNull")]
    pub description: String,
    pub template_type: Option<String>,
    #[serde(default = "default_template_format")]
    pub template_format: String,
}

#[derive(Debug, Deserialize)]
pub struct PromptVersionEnvelope {
    pub data: Option<PromptVersionData>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptMetadata {
    pub model_name: Option<String>,
    pub model_provider: Option<String>,
    pub description: String,
    pub template_type: Option<String>,
    pub template_format: String,
    pub invocation_parameters: IndexMap<String, Json>,
    pub temperature: Option<Number>,
    pub max_tokens: Option<Number>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptTemplate {
    pub template_id: String,
    pub messages: Vec<TemplateMessage>,
    pub metadata: PromptMetadata,
}

fn default_role() -> String {
    "user".to_owned()
}

fn default_template_format() -> String {
    "MUSTACHE".to_owned()
}
