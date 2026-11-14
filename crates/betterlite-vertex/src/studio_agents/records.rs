use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

pub type NativeObject = IndexMap<String, Json>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StudioAgent {
    pub id: Option<String>,
    pub name: Option<String>,
    #[serde(flatten)]
    pub extension: NativeObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentPage {
    pub agents: Vec<NativeObject>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentVersionsPage {
    pub agent_versions: Vec<NativeObject>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentDeleteResult {
    pub name: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWarning {
    MalformedCreateReply,
}
