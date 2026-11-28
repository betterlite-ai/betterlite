use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// LiteLLM's MCPUserEnvVarsRequest; the map holds secrets, so the type carries no Debug at all
pub(crate) struct MCPUserEnvVarsRequest {
    pub(crate) values: BTreeMap<String, String>,
}

// LiteLLM's MCPUserEnvVarSpec: name, a nullable description and a false is_set by default
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct MCPUserEnvVarSpec {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) is_set: bool,
}

// LiteLLM's MCPUserEnvVarsStatus: the identity fields, the blocking specs and the setup link
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct MCPUserEnvVarsStatus {
    pub(crate) server_id: String,
    #[serde(default)]
    pub(crate) server_name: Option<String>,
    #[serde(default)]
    pub(crate) alias: Option<String>,
    #[serde(default)]
    pub(crate) required: Vec<MCPUserEnvVarSpec>,
    #[serde(default)]
    pub(crate) missing_count: u64,
    #[serde(default)]
    pub(crate) setup_url: Option<String>,
}
