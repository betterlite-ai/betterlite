use std::collections::BTreeMap;

use crate::budget_permission_rows::JsonData;
use crate::key_repository::table::TokenValue;
use crate::key_user_writes::json_text::{
    FAILED_TO_SERIALIZE_JSON, JsonEncoder as _, SerdeJsonEncoder,
};

use super::columns::{TableColumn, TeamColumn};

const ENCODER: SerdeJsonEncoder = SerdeJsonEncoder;

// LiteLLM dumps only top-level dict values to JSON text and writes a marker when the dump fails
pub(super) fn jsonify_object<Column>(
    payload: &BTreeMap<Column, TokenValue>,
) -> BTreeMap<Column, TokenValue>
where
    Column: TableColumn,
{
    payload
        .iter()
        .map(|(column, value)| {
            let converted = if let TokenValue::Json(JsonData::Object(_)) = value {
                TokenValue::Json(JsonData::String(dump(value)))
            } else {
                value.clone()
            };
            (*column, converted)
        })
        .collect()
}

// jsonify_team_object additionally dumps the team's list-shaped relation fields to JSON text
pub(super) fn jsonify_team_object(
    payload: &BTreeMap<TeamColumn, TokenValue>,
) -> BTreeMap<TeamColumn, TokenValue> {
    let mut converted = jsonify_object(payload);
    for column in [TeamColumn::MembersWithRoles, TeamColumn::BudgetLimits] {
        dump_list(&mut converted, column);
    }
    converted
}

// the team update branch dumps only members_with_roles, not budget_limits
pub(super) fn jsonify_team_update(
    payload: &BTreeMap<TeamColumn, TokenValue>,
) -> BTreeMap<TeamColumn, TokenValue> {
    let mut converted = jsonify_object(payload);
    dump_list(&mut converted, TeamColumn::MembersWithRoles);
    converted
}

fn dump_list(payload: &mut BTreeMap<TeamColumn, TokenValue>, column: TeamColumn) {
    let Some(value @ TokenValue::Json(JsonData::Array(_))) = payload.get(&column) else {
        return;
    };
    let dumped = TokenValue::Json(JsonData::String(dump(value)));
    payload.insert(column, dumped);
}

fn dump(value: &TokenValue) -> String {
    let TokenValue::Json(json) = value else {
        return String::from(FAILED_TO_SERIALIZE_JSON);
    };
    ENCODER
        .encode(json)
        .unwrap_or_else(|_| String::from(FAILED_TO_SERIALIZE_JSON))
}
