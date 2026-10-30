use serde::{Deserialize, Serialize};

use crate::sources::row_decode::i64_from_int_or_numeric_string;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartRow {
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    pub kind: String,
    pub content: String,
    #[serde(deserialize_with = "i64_from_int_or_numeric_string")]
    pub truncated: i64,
}

mod tests;
