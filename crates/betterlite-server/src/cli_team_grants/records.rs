use std::collections::BTreeMap;

use betterlite_auth::identity_grants::team_grants::{TeamColumnsError, decode_model_aliases};
use betterlite_db::team_rows::aliases::ModelAliases;
use serde::{Deserialize, Serialize};

mod tests;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CliSsoTeamDetail {
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub team_models: Vec<String>,
    pub team_model_aliases: Option<BTreeMap<String, String>>,
}

pub const TEAMLESS_CLI_SSO_TEAM_DETAIL: CliSsoTeamDetail = CliSsoTeamDetail {
    team_id: None,
    team_alias: None,
    team_models: Vec::new(),
    team_model_aliases: None,
};

// _TeamRowGrants: the alias table's single column arrives next to the team's own
#[derive(Clone, Debug, PartialEq)]
pub struct TeamGrantRow {
    pub team_id: String,
    pub team_alias: Option<String>,
    pub models: Vec<String>,
    pub model_aliases: Option<ModelAliases>,
}

pub fn cli_sso_team_detail(row: TeamGrantRow) -> Result<CliSsoTeamDetail, TeamColumnsError> {
    let team_model_aliases = match row.model_aliases.as_ref() {
        Some(column) => decode_model_aliases(column)?,
        None => None,
    };
    Ok(CliSsoTeamDetail {
        team_id: Some(row.team_id),
        team_alias: row.team_alias,
        team_models: row.models,
        team_model_aliases,
    })
}
