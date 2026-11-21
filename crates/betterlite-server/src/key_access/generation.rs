use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::key_access::{KeyGeneration, PersonalKeyGeneration};
use betterlite_db::team_rows::team::TeamBase;

use super::error::KeyAccessError;
use super::team_member::team_member_check;
use crate::key_fields::request::GenerateKeyRequest;
use crate::team_access::caller::TeamCaller;
use crate::team_access::permissions::enforce_group_assignment;

// key_generation_check: a team_id the request names makes a team key, and a proxy admin skips every team rule
pub fn key_generation_check(
    caller: &TeamCaller,
    team: Option<&TeamBase<String>>,
    data: &GenerateKeyRequest,
    route: KeyManagementRoute,
    settings: &KeyGeneration,
) -> Result<(), KeyAccessError> {
    let team_id = data.base.base.team_id.as_ref().and_then(Option::as_deref);
    if caller.team_service_account && team_id != caller.team_id.as_deref() {
        return Err(KeyAccessError::ServiceAccountTeam {
            team_id: caller.team_id.clone(),
        });
    }
    let Some(requested_team) = team_id else {
        return personal_generation_check(caller, data, settings.personal.as_ref());
    };
    let is_admin = caller.user_role == Some(UserRole::ProxyAdmin);
    let Some(found) = team else {
        // LIT-1884: only an unset key_generation_settings lets an admin name a team the database lacks
        if is_admin && !settings.configured {
            return Ok(());
        }
        return Err(KeyAccessError::TeamNotFound {
            team_id: requested_team.to_owned(),
        });
    };
    if is_admin {
        return Ok(());
    }
    team_generation_check(caller, found, data, route, settings)
}

// _team_key_generation_check: membership and role, then required fields, then the access-group opt-in
fn team_generation_check(
    caller: &TeamCaller,
    team: &TeamBase<String>,
    data: &GenerateKeyRequest,
    route: KeyManagementRoute,
    settings: &KeyGeneration,
) -> Result<(), KeyAccessError> {
    let assigned = data.base.base.user_id.as_ref().and_then(Option::as_deref);
    team_member_check(assigned, team, caller, &settings.team, route)?;
    required_param_check(data, settings.team.required_params.as_deref())?;
    let access_groups = data.base.key_fields.access_group_ids.as_ref();
    enforce_group_assignment(caller, Some(team), access_groups.and_then(Option::as_deref))?;
    Ok(())
}

// _personal_key_generation_check: the access-group rule runs before any role setting is read
fn personal_generation_check(
    caller: &TeamCaller,
    data: &GenerateKeyRequest,
    personal: Option<&PersonalKeyGeneration>,
) -> Result<(), KeyAccessError> {
    let access_groups = data.base.key_fields.access_group_ids.as_ref();
    enforce_group_assignment(caller, None, access_groups.and_then(Option::as_deref))?;
    let Some(config) = personal else {
        return Ok(());
    };
    if let Some(allowed) = &config.allowed_user_roles {
        let role = caller.user_role.map(|user_role| user_role.to_string());
        if !role.as_ref().is_some_and(|name| allowed.contains(name)) {
            return Err(KeyAccessError::PersonalRole {
                role,
                allowed: allowed.clone(),
            });
        }
    }
    required_param_check(data, config.required_params.as_deref())
}

// _key_generation_required_param_check reads pydantic's fields_set, so an explicit null counts as present
fn required_param_check(
    data: &GenerateKeyRequest,
    required: Option<&[String]>,
) -> Result<(), KeyAccessError> {
    let Some(params) = required else {
        return Ok(());
    };
    let supplied = data.project()?.supplied;
    match params
        .iter()
        .find(|param| !supplied.contains_key(param.as_str()))
    {
        Some(missing) => Err(KeyAccessError::MissingParam {
            param: missing.clone(),
        }),
        None => Ok(()),
    }
}
