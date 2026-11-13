use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::budget_permission_rows::JsonData;

use crate::management_guards::error::ManagementGuardError;
use crate::management_guards::metadata::has_effect;

pub fn caller_user_id_for_non_admin(principal: &Principal) -> Result<&str, ManagementGuardError> {
    // only a missing user id is rejected: an admin and an empty id both pass
    let user = principal
        .user
        .as_ref()
        .ok_or(ManagementGuardError::ServiceAccountAnalytics)?;
    Ok(user.id.as_str())
}

// metadata has no declared value type, so a request counts when the stored value is nonempty
pub fn check_passthrough_routes(
    principal: &Principal,
    allowed_passthrough_routes: Option<&[JsonData]>,
    metadata: Option<&JsonData>,
    entity: &str,
) -> Result<(), ManagementGuardError> {
    if principal.roles.contains(&Role::PlatformAdmin) {
        return Ok(());
    }
    if allowed_passthrough_routes.is_some_and(|routes| !routes.is_empty()) {
        return Err(ManagementGuardError::PassthroughRoutes {
            entity: entity.to_owned(),
        });
    }
    let flagged = match metadata {
        Some(JsonData::Object(fields)) => fields
            .get("allowed_passthrough_routes")
            .is_some_and(has_effect),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_),
        )
        | None => false,
    };
    if flagged {
        return Err(ManagementGuardError::PassthroughRoutesMetadata {
            entity: entity.to_owned(),
        });
    }
    Ok(())
}

pub fn check_guardrail_opt_out(
    principal: &Principal,
    disable_global_guardrails: Option<bool>,
    metadata: Option<&JsonData>,
    existing_metadata: Option<&JsonData>,
    entity: &str,
) -> Result<(), ManagementGuardError> {
    if principal.roles.contains(&Role::PlatformAdmin) {
        return Ok(());
    }
    let metadata_flag = match metadata {
        Some(JsonData::Object(fields)) => fields
            .get("disable_global_guardrails")
            .is_some_and(has_effect),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_),
        )
        | None => false,
    };
    if disable_global_guardrails != Some(true) && !metadata_flag {
        return Ok(());
    }
    // re-sending a stored exemption is allowed only for the literal boolean true
    let exempted = match existing_metadata {
        Some(JsonData::Object(fields)) => {
            fields.get("disable_global_guardrails") == Some(&JsonData::Bool(true))
        }
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_),
        )
        | None => false,
    };
    if exempted {
        return Ok(());
    }
    Err(ManagementGuardError::GuardrailsDisabled {
        entity: entity.to_owned(),
    })
}
