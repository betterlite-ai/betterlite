use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_db::team_rows::archived::LiteLLMDeletedTeamTable;
use betterlite_db::team_rows::member::Member;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
// ast-grep-ignore: no-long-names - the production operation keeps LiteLLM's function name
use betterlite_server::team_list_query::response::convert_teams_to_response_models;
use betterlite_server::team_request_contracts::team_list::{TeamListEntry, TeamListItem};
use serde::Deserialize;
use serde_json::{Value, json};

// one _convert_teams_to_response_models call: the dumped rows and the page's key counts
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    teams: Vec<TeamRecord>,
    #[serde(default)]
    use_deleted_table: bool,
    #[serde(default)]
    keys_count_by_team: Option<BTreeMap<String, i64>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        // ast-grep-ignore: no-long-names - the same production name
        let converted = convert_teams_to_response_models(
            &input.teams,
            input.use_deleted_table,
            input.keys_count_by_team.as_ref(),
        )
        .map_err(invalid)?;
        let teams: Vec<Value> = converted.iter().map(entry_view).collect();
        Ok(json!({ "teams": teams }))
    }
}

fn invalid<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "ValidationError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

fn entry_view(entry: &TeamListEntry) -> Value {
    match entry {
        TeamListEntry::Enriched(item) => enriched_view(item),
        TeamListEntry::Deleted(row) => deleted_view(row),
        TeamListEntry::Live(team) => live_view(team),
    }
}

fn enriched_view(item: &TeamListItem) -> Value {
    let team = &item.team.fields.base;
    json!({
        "kind": "enriched",
        "team_id": team.team_id,
        "team_alias": team.team_alias,
        "members_with_roles": members_view(&team.members_with_roles),
        "members_count": item.members_count,
        "keys_count": item.keys_count,
        "access_group_models": item.access_group_models,
        "access_group_mcp_server_ids": item.access_group_mcp_server_ids,
        "access_group_agent_ids": item.access_group_agent_ids,
    })
}

fn deleted_view(row: &LiteLLMDeletedTeamTable) -> Value {
    let team = &row.team.fields.base;
    json!({
        "kind": "deleted",
        "team_id": team.team_id,
        "team_alias": team.team_alias,
        "members_with_roles": members_view(&team.members_with_roles),
        "id": row.audit.id,
        "deleted_by": row.audit.deleted_by,
        "deleted_by_api_key": row.audit.deleted_by_api_key,
        "litellm_changed_by": row.audit.litellm_changed_by,
    })
}

fn live_view(team: &LiteLLMTeamTable) -> Value {
    let base = &team.fields.base;
    json!({
        "kind": "live",
        "team_id": base.team_id,
        "team_alias": base.team_alias,
        "members_with_roles": members_view(&base.members_with_roles),
    })
}

fn members_view(members: &[Member]) -> Vec<Value> {
    members
        .iter()
        .map(|member| json!({ "user_id": member.base.user_id, "role": member.role }))
        .collect()
}
