use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_db::team_rows::archived::LiteLLMDeletedTeamTable;

use crate::team_request_contracts::team_list::{TeamListEntry, TeamListItem};

use super::TeamListQueryError;

// ast-grep-ignore: no-long-names - LiteLLM's name, and a Prisma row is already its model_dump()
pub fn convert_teams_to_response_models(
    teams: &[TeamRecord],
    use_deleted_table: bool,
    keys_count_by_team: Option<&BTreeMap<String, i64>>,
) -> Result<Vec<TeamListEntry>, TeamListQueryError> {
    let mut converted = Vec::with_capacity(teams.len());
    for record in teams {
        converted.push(if use_deleted_table {
            deleted_entry(record)?
        } else {
            live_entry(record, keys_count_by_team)?
        });
    }
    Ok(converted)
}

fn fields_of(record: &TeamRecord) -> BTreeMap<String, JsonData> {
    record
        .fields
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn deleted_entry(record: &TeamRecord) -> Result<TeamListEntry, TeamListQueryError> {
    LiteLLMDeletedTeamTable::try_from(JsonData::Object(fields_of(record)))
        .map(TeamListEntry::Deleted)
        .map_err(TeamListQueryError::from)
}

fn live_entry(
    record: &TeamRecord,
    keys_count_by_team: Option<&BTreeMap<String, i64>>,
) -> Result<TeamListEntry, TeamListQueryError> {
    let mut fields = fields_of(record);
    let members_count = roster_count(&mut fields)?;
    let keys_count = key_count(keys_count_by_team, &fields);
    fields.insert(
        "members_count".to_owned(),
        JsonData::Number(serde_json::Number::from(members_count)),
    );
    fields.insert(
        "keys_count".to_owned(),
        JsonData::Number(serde_json::Number::from(keys_count)),
    );
    let item = TeamListItem::try_from(&JsonData::Object(fields))?;
    Ok(TeamListEntry::Enriched(item))
}

// a non-list members_with_roles becomes [], and its length is the only count
fn roster_count(fields: &mut BTreeMap<String, JsonData>) -> Result<i64, TeamListQueryError> {
    let members = match fields.remove("members_with_roles") {
        Some(JsonData::Array(members)) => members,
        Some(_) | None => Vec::new(),
    };
    let count =
        i64::try_from(members.len()).map_err(|source| TeamListQueryError::Count { source })?;
    fields.insert("members_with_roles".to_owned(), JsonData::Array(members));
    Ok(count)
}

// counts.get(team_dict.get("team_id") or "", 0): a missing or empty ID reads the "" entry
fn key_count(
    keys_count_by_team: Option<&BTreeMap<String, i64>>,
    fields: &BTreeMap<String, JsonData>,
) -> i64 {
    let team_id = match fields.get("team_id") {
        Some(JsonData::String(team_id)) => team_id.as_str(),
        Some(_) | None => "",
    };
    keys_count_by_team
        .and_then(|counts| counts.get(team_id))
        .copied()
        .unwrap_or(0)
}
