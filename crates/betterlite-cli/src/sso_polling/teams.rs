use serde::Deserialize;
use serde_json::value::RawValue;

use crate::credentials::contracts::CliTeam;

use super::error::PollingError;
use super::wire;

#[derive(Deserialize)]
struct TeamDetail {
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    team_alias: Option<String>,
}

pub fn normalize_teams(
    teams: &[String],
    team_details: Option<&RawValue>,
) -> Result<Vec<CliTeam>, PollingError> {
    if let Some(raw) = team_details {
        let details: Vec<TeamDetail> = wire::decode(raw.get().as_bytes())
            .map_err(|source| PollingError::TeamDetails { source })?;
        if !details.is_empty() {
            return Ok(normalized_details(details));
        }
    }
    Ok(teams
        .iter()
        .map(|team_id| CliTeam {
            team_id: Some(team_id.clone()),
            team_alias: None,
            models: Vec::new(),
            max_budget: None,
        })
        .collect())
}

fn normalized_details(details: Vec<TeamDetail>) -> Vec<CliTeam> {
    let mut teams = Vec::new();
    for detail in details {
        let Some(team_id) = detail.team_id.or(detail.id) else {
            continue;
        };
        teams.push(CliTeam {
            team_id: Some(team_id),
            team_alias: detail.team_alias,
            models: Vec::new(),
            max_budget: None,
        });
    }
    teams
}
