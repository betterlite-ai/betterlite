use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use serde::Serialize;

const METADATA_FIELD: &str = "metadata";
const USER_FIELD: &str = "user_api_key_user_id";
const TEAM_FIELD: &str = "user_api_key_team_id";

// the ("user", team_id) pair a spend-log row or a cold-storage payload records as its owner
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct OwnerPair {
    pub user: Option<String>,
    pub team_id: Option<String>,
}

// _cold_storage_payload_owner: a missing or non-string field counts as absent, so the check fails closed
#[must_use]
pub fn cold_payload_owner(payload: &JsonData) -> OwnerPair {
    let metadata = match payload {
        JsonData::Object(fields) => fields.get(METADATA_FIELD),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => None,
    };
    let Some(JsonData::Object(owner)) = metadata else {
        return OwnerPair {
            user: None,
            team_id: None,
        };
    };
    OwnerPair {
        user: string_field(owner, USER_FIELD),
        team_id: string_field(owner, TEAM_FIELD),
    }
}

fn string_field(fields: &BTreeMap<String, JsonData>, name: &str) -> Option<String> {
    fields.get(name).and_then(|value| match value {
        JsonData::String(text) => Some(text.clone()),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    })
}
