use serde::{Deserialize, Serialize};

use super::json::Object;

const INTERACTION_OBJECT: Option<&str> = Some("interaction");

fn default_object() -> Option<String> {
    INTERACTION_OBJECT.map(str::to_owned)
}

const fn default_success() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InteractionsApiResponse {
    pub id: Option<String>,
    #[serde(default = "default_object")]
    pub object: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub status: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    // the legacy schema (Api-Revision 2026-05-07) carries outputs, the new one carries steps
    pub outputs: Option<Vec<Object>>,
    pub steps: Option<Vec<Object>>,
    pub usage: Option<Object>,
    #[serde(flatten)]
    pub extra: Object,
    #[serde(skip)]
    pub hidden_params: Object,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InteractionsStreamChunk {
    pub event_type: Option<String>,
    pub id: Option<String>,
    #[serde(default = "default_object")]
    pub object: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub status: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub outputs: Option<Vec<Object>>,
    pub steps: Option<Vec<Object>>,
    pub usage: Option<Object>,
    pub delta: Option<Object>,
    pub index: Option<i64>,
    pub step: Option<Object>,
    pub interaction_id: Option<String>,
    pub interaction: Option<Object>,
    #[serde(flatten)]
    pub extra: Object,
    #[serde(skip)]
    pub hidden_params: Object,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DeleteInteractionResult {
    #[serde(default = "default_success")]
    pub success: bool,
    pub id: Option<String>,
    #[serde(flatten)]
    pub extra: Object,
    #[serde(skip)]
    pub hidden_params: Object,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CancelInteractionResult {
    pub id: Option<String>,
    pub status: Option<String>,
    #[serde(flatten)]
    pub extra: Object,
    #[serde(skip)]
    pub hidden_params: Object,
}
