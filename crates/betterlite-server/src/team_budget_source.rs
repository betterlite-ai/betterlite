use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeamMemberBudgetSource {
    #[serde(rename = "team_default")]
    TeamDefault,
    #[serde(rename = "custom")]
    Custom,
    #[serde(rename = "none")]
    NoBudget,
}

// an empty string is a Python str like any other, so it is a default id
#[must_use]
pub fn team_default_budget_id(metadata: Option<&BTreeMap<String, JsonData>>) -> Option<&str> {
    match metadata?.get("team_member_budget_id")? {
        JsonData::String(budget_id) => Some(budget_id.as_str()),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}

// the member id and the default compare as exact strings, so "" equals ""
#[must_use]
pub fn member_budget_source(
    budget_id: Option<&str>,
    team_default_budget_id: Option<&str>,
) -> TeamMemberBudgetSource {
    if budget_id.is_some() && budget_id != team_default_budget_id {
        return TeamMemberBudgetSource::Custom;
    }
    if team_default_budget_id.is_some() {
        return TeamMemberBudgetSource::TeamDefault;
    }
    TeamMemberBudgetSource::NoBudget
}
