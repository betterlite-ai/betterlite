use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct CachedEntry(pub serde_json::Value);

#[path = "literal.rs"]
pub mod literal;
