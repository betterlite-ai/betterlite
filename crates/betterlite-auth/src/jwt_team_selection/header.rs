use std::collections::{BTreeMap, BTreeSet};

use crate::auth_team_lookup::alias::get_team_object_by_alias;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::{TeamLookup, TeamRequest, TeamSource};
use crate::auth_team_lookup::team::get_team_object;

use super::error::TeamSelectionError;

const TEAM_HEADER: &str = "x-litellm-team-id";

// the team the x-litellm-team-id header names, before B891's membership check turns it into request context
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderTeam {
    pub header_value: String,
    pub team_id: String,
}

// _team_header_value: the header name matches without regard to case and its value is returned as sent
#[must_use]
pub fn team_header_value(request_headers: Option<&BTreeMap<String, String>>) -> Option<String> {
    let headers = request_headers?;
    headers
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case(TEAM_HEADER))
        .map(|(_name, value)| value.clone())
}

// resolve_team_from_header: an allowed id costs no lookup; otherwise the alias must resolve inside the allowed ids
pub async fn resolve_team_from_header(
    request_headers: Option<&BTreeMap<String, String>>,
    allowed_team_ids: &BTreeSet<String>,
    fallback_to_db_teams: bool,
    lookup: &TeamLookup,
) -> Result<Option<HeaderTeam>, TeamSelectionError> {
    // LiteLLM tests the value for truthiness here, so an empty header selects nothing
    let Some(header_value) = team_header_value(request_headers).filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if allowed_team_ids.contains(&header_value) {
        return Ok(Some(HeaderTeam {
            header_value: header_value.clone(),
            team_id: header_value,
        }));
    }
    if fallback_to_db_teams {
        return provisional_team(header_value, lookup).await;
    }
    match team_id_by_alias(lookup, &header_value).await? {
        Some(team_id) if allowed_team_ids.contains(&team_id) => Ok(Some(HeaderTeam {
            header_value,
            team_id,
        })),
        _ => Err(TeamSelectionError::header_team_not_allowed(
            header_value,
            allowed_team_ids,
        )),
    }
}

// the DB fallback accepts a header outside the claims provisionally, by id or unique alias
async fn provisional_team(
    header_value: String,
    lookup: &TeamLookup,
) -> Result<Option<HeaderTeam>, TeamSelectionError> {
    let request = TeamRequest {
        team_id: &header_value,
        source: TeamSource::CacheThenDatabase,
        team_id_upsert: false,
    };
    match get_team_object(lookup, request).await {
        Ok(_team) => Ok(Some(HeaderTeam {
            header_value: header_value.clone(),
            team_id: header_value,
        })),
        // only a provably absent row falls through to the alias lookup
        Err(TeamLookupError::TeamNotFound { .. }) => {
            match team_id_by_alias(lookup, &header_value).await? {
                Some(team_id) => Ok(Some(HeaderTeam {
                    header_value,
                    team_id,
                })),
                None => Err(TeamSelectionError::HeaderTeamMembershipDenied { header_value }),
            }
        }
        // every other read failure keeps the membership denial, which reveals neither existence nor an id
        Err(error) if error.http().is_some() => {
            Err(TeamSelectionError::HeaderTeamMembershipDenied { header_value })
        }
        Err(error) => Err(TeamSelectionError::Lookup(Box::new(error))),
    }
}

// _team_id_by_alias: no database, a 4xx and an ambiguous alias all answer None; a 5xx is re-raised
async fn team_id_by_alias(
    lookup: &TeamLookup,
    team_alias: &str,
) -> Result<Option<String>, TeamSelectionError> {
    if lookup.database.is_none() {
        return Ok(None);
    }
    match get_team_object_by_alias(lookup, team_alias).await {
        Ok(team) => Ok(Some(team.team.fields.base.team_id.clone())),
        Err(error) => {
            let client_error = error
                .http()
                .is_some_and(|(status_code, _detail)| status_code < 500);
            if client_error {
                Ok(None)
            } else {
                Err(TeamSelectionError::Lookup(Box::new(error)))
            }
        }
    }
}
