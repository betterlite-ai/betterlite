use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

use super::error::{TeamFailure, TeamLookupError};
use super::lookup::{TeamDatabase, TeamLookup, TeamRequest, TeamSource, attach_permission};
use super::ports::TeamSeed;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use crate::user_roles::UserRole;

// auth_checks.py get_team_object: the cache unless the writer is asked for, then the throttled read
pub async fn get_team_object(
    lookup: &TeamLookup,
    request: TeamRequest<'_>,
) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
    let Some(database) = lookup.database.as_ref() else {
        return Err(TeamLookupError::NoDatabase);
    };
    let team_id = request.team_id;
    let key = format!("team_id:{team_id}");
    if request.source != TeamSource::Writer {
        let cached = lookup
            .cache
            .read_team(&key)
            .await
            .map_err(|error| TeamLookupError::CacheRead(Box::new(error)))?;
        if let Some(team) = cached {
            return Ok(team);
        }
        if request.source == TeamSource::CacheOnly {
            return Err(TeamLookupError::NotInCache {
                team_id: team_id.to_owned(),
            });
        }
    }
    match database_team(lookup, database, request, &key).await {
        Ok(Some(team)) => Ok(team),
        Ok(None) => Err(TeamLookupError::TeamNotFound {
            team_id: team_id.to_owned(),
        }),
        Err(cause) => Err(TeamLookupError::TeamUnavailable {
            team_id: team_id.to_owned(),
            cause: Box::new(cause),
        }),
    }
}

// _get_team_object_from_user_api_key_cache: check_db_only still waits out a recent miss
async fn database_team(
    lookup: &TeamLookup,
    database: &TeamDatabase,
    request: TeamRequest<'_>,
    key: &str,
) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamFailure> {
    if !lookup
        .cadence
        .should_check_db(key, i64::from(lookup.in_memory_ttl))
    {
        return Err(TeamFailure::Throttled);
    }
    let use_writer = request.source == TeamSource::Writer;
    let Some(mut team) = team_db_check(lookup, database, request, use_writer).await? else {
        return Ok(None);
    };
    attach_permission(lookup, database, &mut team, use_writer)
        .await
        .map_err(TeamFailure::Permission)?;
    lookup
        .cache
        .cache_team(request.team_id, &mut team)
        .await
        .map_err(|error| TeamFailure::Cache(Box::new(error)))?;
    let wire = serde_json::to_value(&team).map_err(TeamFailure::Cadence)?;
    let record = serde_json::from_value::<JsonData>(wire).map_err(TeamFailure::Cadence)?;
    lookup.cadence.update(key, Some(record));
    Ok(Some(team))
}

// _get_team_db_check: the grant-relation read, then team_id_upsert's creation when the row is absent
async fn team_db_check(
    lookup: &TeamLookup,
    database: &TeamDatabase,
    request: TeamRequest<'_>,
    use_writer: bool,
) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamFailure> {
    let found = database
        .store
        .find_team(request.team_id, use_writer)
        .await
        .map_err(|error| TeamFailure::Store(Box::new(error)))?;
    if let Some(row) = found {
        let team = LiteLLMTeamTableCachedObj::try_from(row)
            .map_err(|error| TeamFailure::Row(Box::new(error)))?;
        return Ok(Some(team));
    }
    if !request.team_id_upsert {
        return Ok(None);
    }
    let provisioner = lookup
        .provisioner
        .as_deref()
        .ok_or(TeamFailure::NoProvisioner)?;
    let team = provisioner
        .create(admin_seed(request.team_id)?)
        .await
        .map_err(|error| TeamFailure::Provision(Box::new(error)))?;
    Ok(Some(LiteLLMTeamTableCachedObj {
        team,
        last_refreshed_at: None,
    }))
}

// NewTeamRequest(team_id=...) with UserAPIKeyAuth(user_role=PROXY_ADMIN), new_team's synthetic caller
fn admin_seed(team_id: &str) -> Result<TeamSeed, TeamFailure> {
    let mut fields = BTreeMap::new();
    let role = UserRole::ProxyAdmin.to_string();
    fields.insert("user_role".to_owned(), JsonData::String(role));
    let principal = UserAPIKeyAuth::construct(AuthInput::Dict(fields))
        .map_err(|error| TeamFailure::Principal(Box::new(error)))?;
    Ok(TeamSeed {
        team_id: team_id.to_owned(),
        principal,
    })
}
