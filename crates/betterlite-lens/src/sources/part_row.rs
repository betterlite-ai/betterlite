use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartRow {
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    pub kind: String,
    pub content: String,
    pub truncated: i64,
}

mod tests;
