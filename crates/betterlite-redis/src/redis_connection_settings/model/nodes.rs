mod tests;

use serde::{Deserialize, Serialize};

use super::scalars::{IntOrText, Scalar};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClusterNodeConfig {
    pub host: String,
    pub port: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClusterNodeInput {
    pub host: String,
    pub port: IntOrText,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum SentinelNodeInput {
    Object { host: String, port: IntOrText },
    Tuple((String, IntOrText)),
}

pub type NodesInput<T> = Scalar<Vec<T>>;

impl<T> Scalar<Vec<T>> {
    #[must_use]
    pub const fn is_truthy(&self) -> bool {
        match self {
            Self::Value(nodes) => !nodes.is_empty(),
            Self::Text(text) => !text.is_empty(),
        }
    }
}

pub type StartupNodesInput = NodesInput<ClusterNodeInput>;
pub type SentinelNodesInput = NodesInput<SentinelNodeInput>;

pub trait NodeInput {
    type Resolved;
    fn into_parts(self) -> (String, IntOrText);
    fn from_parts(host: String, port: i64) -> Self::Resolved;
}

impl NodeInput for ClusterNodeInput {
    type Resolved = ClusterNodeConfig;

    fn into_parts(self) -> (String, IntOrText) {
        (self.host, self.port)
    }

    fn from_parts(host: String, port: i64) -> Self::Resolved {
        ClusterNodeConfig { host, port }
    }
}

impl NodeInput for SentinelNodeInput {
    type Resolved = (String, i64);

    fn into_parts(self) -> (String, IntOrText) {
        match self {
            Self::Object { host, port } | Self::Tuple((host, port)) => (host, port),
        }
    }

    fn from_parts(host: String, port: i64) -> Self::Resolved {
        (host, port)
    }
}
