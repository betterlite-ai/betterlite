use serde::{Deserialize, Serialize};

use crate::models::execution::ExecutionSource;

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
    #[serde(
        deserialize_with = "serde_with::As::<serde_with::PickFirst<(serde_with::Same, serde_with::DisplayFromStr)>>::deserialize"
    )]
    pub span_count: i64,
    #[serde(
        deserialize_with = "serde_with::As::<serde_with::PickFirst<(serde_with::Same, serde_with::DisplayFromStr)>>::deserialize"
    )]
    pub root_seen: i64,
    #[serde(
        deserialize_with = "serde_with::As::<serde_with::PickFirst<(serde_with::Same, serde_with::DisplayFromStr)>>::deserialize"
    )]
    pub eligible: i64,
    #[serde(
        default,
        deserialize_with = "serde_with::As::<serde_with::PickFirst<(serde_with::Same, serde_with::DisplayFromStr)>>::deserialize"
    )]
    pub selected: i64,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub attributes: Vec<(String, String)>,
}

mod tests;
