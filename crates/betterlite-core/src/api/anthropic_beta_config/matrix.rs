use std::str;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::error::{BetaConfigError, BetaShapeError};
use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};

const PROVIDER_KEYS: [&str; 5] = [
    "anthropic",
    "azure_ai",
    "bedrock",
    "bedrock_converse",
    "vertex_ai",
];

// Only the top level is checked, so provider values stay dynamic nodes that B960 reads one by one
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BetaNode {
    Null,
    Flag(bool),
    Number(serde_json::Number),
    Text(String),
    List(Vec<Self>),
    Map(IndexMap<String, Self>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BetaMatrix(pub IndexMap<String, BetaNode>);

impl BetaMatrix {
    // One hop only: an alias that points at another alias stays at that alias
    #[must_use]
    pub fn provider_name<'name>(&'name self, provider: &'name str) -> &'name str {
        match self.0.get("provider_aliases") {
            Some(BetaNode::Map(aliases)) => match aliases.get(provider) {
                Some(BetaNode::Text(canonical)) => canonical,
                _ => provider,
            },
            _ => provider,
        }
    }
}

pub fn decode_beta_matrix(bytes: &[u8], limits: ApiLimits) -> Result<BetaNode, BetaConfigError> {
    limits
        .check(ApiBoundariesInput::Json(JsonBoundary {
            known_bytes: bytes.len(),
        }))
        .map_err(BetaConfigError::TooLarge)?;
    let text = str::from_utf8(bytes).map_err(BetaConfigError::Utf8)?;
    serde_json::from_str(text).map_err(BetaConfigError::Json)
}

pub fn validate_beta_matrix(node: BetaNode) -> Result<BetaMatrix, BetaShapeError> {
    let json_type = match node {
        BetaNode::Map(map) => return check_providers(map),
        BetaNode::Null => "null",
        BetaNode::Flag(_) => "boolean",
        BetaNode::Number(_) => "number",
        BetaNode::Text(_) => "string",
        BetaNode::List(_) => "array",
    };
    Err(BetaShapeError::NotAnObject { json_type })
}

fn check_providers(map: IndexMap<String, BetaNode>) -> Result<BetaMatrix, BetaShapeError> {
    if map.is_empty() {
        return Err(BetaShapeError::Empty);
    }
    if !PROVIDER_KEYS.iter().any(|key| map.contains_key(*key)) {
        return Err(BetaShapeError::MissingProviderKeys);
    }
    Ok(BetaMatrix(map))
}
