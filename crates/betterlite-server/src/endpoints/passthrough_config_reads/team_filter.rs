use std::collections::BTreeMap;
use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_repository::repository::TeamQueryError;
use thiserror::Error;

use super::ports::TeamLookup;
use super::record::PassThroughGenericEndpoint;

pub const ALLOWED_ROUTES_KEY: &str = "allowed_passthrough_routes";

#[derive(Debug, Error)]
pub enum TeamFilterError {
    #[error("the team was not found")]
    TeamNotFound,
    #[error("the metadata of team {team_id} is not an object")]
    Metadata { team_id: String },
    #[error("the allowed pass-through routes of team {team_id} are not a list")]
    AllowedRoutes { team_id: String },
    #[error("the team lookup failed")]
    Lookup(#[from] TeamQueryError),
}

// LiteLLM raises 404 for a missing team and otherwise filters by the routes the team's metadata allows, when it names any
pub async fn filter_endpoints_by_team(
    teams: &dyn TeamLookup,
    team_id: &str,
    endpoints: &[PassThroughGenericEndpoint],
) -> Result<Vec<PassThroughGenericEndpoint>, TeamFilterError> {
    let Some(metadata) = teams.find_team_metadata(team_id).await? else {
        return Err(TeamFilterError::TeamNotFound);
    };
    match metadata {
        JsonData::Null => Ok(endpoints.to_vec()),
        JsonData::Object(fields) => allowed_by_metadata(&fields, team_id, endpoints),
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Array(_) => {
            Err(TeamFilterError::Metadata {
                team_id: team_id.to_owned(),
            })
        }
    }
}

fn allowed_by_metadata(
    fields: &BTreeMap<String, JsonData>,
    team_id: &str,
    endpoints: &[PassThroughGenericEndpoint],
) -> Result<Vec<PassThroughGenericEndpoint>, TeamFilterError> {
    let routes = match fields.get(ALLOWED_ROUTES_KEY) {
        None | Some(JsonData::Null) => return Ok(endpoints.to_vec()),
        Some(JsonData::Array(routes)) => routes,
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Object(_),
        ) => {
            return Err(TeamFilterError::AllowedRoutes {
                team_id: team_id.to_owned(),
            });
        }
    };
    let paths: BTreeSet<&str> = routes
        .iter()
        .filter_map(|route| match route {
            JsonData::String(path) => Some(path.as_str()),
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_) => None,
        })
        .collect();
    Ok(endpoints
        .iter()
        .filter(|endpoint| paths.contains(endpoint.path.as_str()))
        .cloned()
        .collect())
}
