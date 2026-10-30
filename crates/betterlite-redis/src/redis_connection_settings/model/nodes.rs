mod tests;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ClusterNodeConfig {
    pub host: String,
    pub port: i64,
}

// A caller may hold a parsed node list already; the environment only ever offers JSON text.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StartupNodesInput {
    Nodes(Vec<ClusterNodeConfig>),
    Text(String),
}

impl StartupNodesInput {
    #[must_use]
    pub const fn is_truthy(&self) -> bool {
        match self {
            Self::Nodes(nodes) => !nodes.is_empty(),
            Self::Text(text) => !text.is_empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SentinelNodesInput {
    Nodes(Vec<(String, i64)>),
    Text(String),
}

impl SentinelNodesInput {
    #[must_use]
    pub const fn is_truthy(&self) -> bool {
        match self {
            Self::Nodes(nodes) => !nodes.is_empty(),
            Self::Text(text) => !text.is_empty(),
        }
    }
}
