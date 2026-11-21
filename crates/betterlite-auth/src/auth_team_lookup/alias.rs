use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use tracing::error;

use super::error::{TeamFailure, TeamLookupError};
use super::lookup::{TeamDatabase, TeamLookup, attach_permission};

// auth_checks.py get_team_object_by_alias: the alias cache, then one unique match by team_alias
pub async fn get_team_object_by_alias(
    lookup: &TeamLookup,
    team_alias: &str,
) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
    let Some(database) = lookup.database.as_ref() else {
        return Err(TeamLookupError::NoDatabase);
    };
    let cache_key = format!("team_alias:{team_alias}");
    let cached = lookup
        .cache
        .read_team(&cache_key)
        .await
        .map_err(|error| TeamLookupError::CacheRead(Box::new(error)))?;
    if let Some(team) = cached {
        return Ok(team);
    }
    let failed = |cause: TeamFailure| alias_failure(team_alias, cause);
    let mut teams = database
        .store
        .find_by_alias(team_alias)
        .await
        .map_err(|error| failed(TeamFailure::Store(Box::new(error))))?;
    if teams.len() > 1 {
        return Err(TeamLookupError::AliasAmbiguous {
            team_alias: team_alias.to_owned(),
        });
    }
    let Some(row) = teams.pop() else {
        return Err(TeamLookupError::AliasNotFound {
            team_alias: team_alias.to_owned(),
        });
    };
    let team = LiteLLMTeamTableCachedObj::try_from(row)
        .map_err(|error| failed(TeamFailure::Row(Box::new(error))))?;
    cache_alias_match(lookup, database, team, &cache_key)
        .await
        .map_err(failed)
}

// every failure but the two HTTP answers above logs with its traceback and becomes the 500
fn alias_failure(team_alias: &str, cause: TeamFailure) -> TeamLookupError {
    error!(team_alias, error = %cause, "Error looking up team by alias: {team_alias}");
    TeamLookupError::AliasFailure {
        team_alias: team_alias.to_owned(),
        cause: Box::new(cause),
    }
}

// the alias entry first, then the team_id entry, both with DEFAULT_IN_MEMORY_TTL rather than the team-cache policy
async fn cache_alias_match(
    lookup: &TeamLookup,
    database: &TeamDatabase,
    mut team: LiteLLMTeamTableCachedObj,
    cache_key: &str,
) -> Result<LiteLLMTeamTableCachedObj, TeamFailure> {
    attach_permission(lookup, database, &mut team, false)
        .await
        .map_err(TeamFailure::Permission)?;
    let ttl = f64::from(lookup.in_memory_ttl);
    let team_id_key = format!("team_id:{}", team.team.fields.base.team_id);
    for key in [cache_key, team_id_key.as_str()] {
        lookup
            .cache
            .write_team(key, &team, ttl)
            .await
            .map_err(|error| TeamFailure::Cache(Box::new(error)))?;
    }
    Ok(team)
}
