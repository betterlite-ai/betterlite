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
pub enum NodesInput<T> {
    Nodes(Vec<T>),
    Text(String),
}

impl<T> NodesInput<T> {
    #[must_use]
    pub const fn is_truthy(&self) -> bool {
        match self {
            Self::Nodes(nodes) => !nodes.is_empty(),
            Self::Text(text) => !text.is_empty(),
        }
    }
}

pub type StartupNodesInput = NodesInput<ClusterNodeConfig>;
pub type SentinelNodesInput = NodesInput<(String, i64)>;
