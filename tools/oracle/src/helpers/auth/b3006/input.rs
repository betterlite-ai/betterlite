use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) uptime_seconds: f64,
    #[serde(default)]
    pub(super) memory: BTreeMap<String, JsonData>,
    pub(super) calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Call {
    Key(CheckCall),
    User(CheckCall),
    EndUser(CheckCall),
    Team(TeamCall),
    Fallback(FallbackCall),
    Charge(Box<ChargeCall>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckCall {
    pub(super) entity_id: String,
    #[serde(default)]
    pub(super) key_alias: Option<String>,
    pub(super) model: String,
    #[serde(default)]
    pub(super) budget: Option<BTreeMap<String, JsonData>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TeamCall {
    pub(super) team_id: String,
    pub(super) model: String,
    #[serde(default)]
    pub(super) budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(super) key_budget: Option<BTreeMap<String, JsonData>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FallbackCall {
    pub(super) entity_id: String,
    #[serde(default)]
    pub(super) key_alias: Option<String>,
    pub(super) model: String,
    #[serde(default)]
    pub(super) budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(super) fallbacks: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChargeCall {
    #[serde(default)]
    pub(super) model_group: Option<String>,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) response_cost: Option<serde_json::Number>,
    #[serde(default)]
    pub(super) key_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(super) team_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(super) user_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(super) end_user_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(super) key_hash: Option<String>,
    #[serde(default)]
    pub(super) team_id: Option<String>,
    #[serde(default)]
    pub(super) user_id: Option<String>,
    #[serde(default)]
    pub(super) end_user: Option<String>,
    #[serde(default)]
    pub(super) end_user_id: Option<String>,
    #[serde(default)]
    pub(super) call_type: Option<String>,
    #[serde(default)]
    pub(super) batch_id: Option<String>,
}
