use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use serde::Serialize;

use super::{MemberContractError, field, json_object};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeleteTeamRequest {
    pub team_ids: Vec<String>,
}

impl DeleteTeamRequest {
    pub fn from_value(value: &Json) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let team_ids: Vec<String> = field(object, "team_ids", Json::Null)?;
        Ok(Self {
            team_ids: distinct_team_ids(team_ids),
        })
    }
}

fn distinct_team_ids(team_ids: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut kept = Vec::new();
    for team_id in team_ids {
        let first = seen.insert(team_id.clone());
        if first {
            kept.push(team_id);
        }
    }
    kept
}

mod tests;
