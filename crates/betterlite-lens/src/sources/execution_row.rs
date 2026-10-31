use serde::{Deserialize, Serialize};

use crate::models::execution::ExecutionSource;
use crate::sources::row_decode::i64_from_int_or_numeric_string;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRow {
    #[serde(default)]
    pub selection_key: String,
    pub source: ExecutionSource,
    pub trace_id: String,
    #[serde(default)]
    pub trace_ref: String,
    pub team_id: String,
    pub name: String,
    pub start_time: String,
    #[serde(deserialize_with = "i64_from_int_or_numeric_string")]
    pub span_count: i64,
    #[serde(deserialize_with = "i64_from_int_or_numeric_string")]
    pub root_seen: i64,
    #[serde(deserialize_with = "i64_from_int_or_numeric_string")]
    pub eligible: i64,
    #[serde(default, deserialize_with = "i64_from_int_or_numeric_string")]
    pub selected: i64,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub attributes: Vec<(String, String)>,
}

mod tests;
