use std::collections::BTreeMap;

use betterlite_bedrock::event_frames::beta::BetaHeader;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model: String,
    pub(crate) model_cost: Json,
    pub(crate) tools: Vec<Json>,
    pub(crate) headers: Option<BTreeMap<String, BetaHeader>>,
    pub(crate) params: IndexMap<String, Json>,
}
