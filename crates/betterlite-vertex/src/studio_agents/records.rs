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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentWarning {
    MalformedCreateReply,
}

impl AgentWarning {
    #[must_use]
    pub const fn subject(self) -> &'static str {
        match self {
            Self::MalformedCreateReply => "malformed_create_reply",
        }
    }
}
