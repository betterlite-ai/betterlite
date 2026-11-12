use serde::{Deserialize, Serialize};

use super::config::GenerationConfig;
use super::content::Content;
use super::enums::{ResponseModality, ThinkingSummaries};
use super::json::Json;
use super::status::InteractionStatus;
use super::tools::Tool;
use super::usage::Usage;

fn default_object() -> String {
    "interaction".to_owned()
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Interaction {
    pub model: Option<String>,
    pub agent: Option<String>,
    pub id: String,
    pub status: InteractionStatus,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub outputs: Option<Vec<Content>>,
    pub system_instruction: Option<String>,
    pub tools: Option<Vec<Tool>>,
    pub background: Option<bool>,
    #[serde(default = "default_object")]
    pub object: String,
    pub usage: Option<Usage>,
    pub response_modalities: Option<Vec<ResponseModality>>,
    // response_format is the OpenAPI schema the caller supplied, kept as given
    pub response_format: Option<Json>,
    pub response_mime_type: Option<String>,
    pub previous_interaction_id: Option<String>,
    pub input: Option<InteractionInput>,
    pub generation_config: Option<GenerationConfig>,
    pub agent_config: Option<AgentConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum AgentConfig {
    #[serde(rename = "dynamic")]
    Dynamic,
    #[serde(rename = "deep-research")]
    DeepResearch(DeepResearchAgentConfig),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DeepResearchAgentConfig {
    pub thinking_summaries: Option<ThinkingSummaries>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum InteractionInput {
    Text(String),
    Content(Content),
    Turn(Turn),
    ContentList(Vec<Content>),
    TurnList(Vec<Turn>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Turn {
    pub role: Option<String>,
    pub content: Option<TurnContent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TurnContent {
    Text(String),
    Content(Vec<Content>),
}
