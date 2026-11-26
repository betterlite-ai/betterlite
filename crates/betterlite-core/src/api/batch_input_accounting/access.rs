use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_model_patterns::ALL_TEAM_MODELS;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexSet;

use super::error::BatchHttpError;
use super::ports::{BatchAccessCheck, BatchKeyGrant, BatchModelAccess, BatchTeamLookup};
use crate::api::batch_limit_descriptors::fields::BatchPrincipalFields;

// Why enforcement stopped: a field read, an auth-slice failure, or the 403/HTTPException it re-raises
#[derive(Debug)]
pub(super) enum AccessFailure<AccessError> {
    Fields(serde_json::Error),
    Source(AccessError),
    Http(BatchHttpError),
}

// _enforce_batch_file_model_access: the upload target's names win over the row models, then each name is checked
pub(super) async fn enforce<Access>(
    access: &Access,
    caller: &UserAPIKeyAuth,
    row_models: &IndexSet<String>,
    target_models: Option<&[String]>,
) -> Result<(), AccessFailure<Access::Error>>
where
    Access: BatchModelAccess,
{
    let models: Vec<&str> = match target_models {
        Some(names) if !names.is_empty() => names.iter().map(String::as_str).collect(),
        _ => row_models.iter().map(String::as_str).collect(),
    };
    if models.is_empty() {
        return Ok(());
    }
    let fields: BatchPrincipalFields = serde_json::to_value(caller)
        .and_then(serde_json::from_value)
        .map_err(AccessFailure::Fields)?;
    let team = if wants_team(access, caller, &fields) {
        let team_id = account_limits(caller).team_id.unwrap_or_default();
        match access.team(team_id).await.map_err(AccessFailure::Source)? {
            BatchTeamLookup::Loaded(team) => Some(team),
            BatchTeamLookup::Failed => return Err(AccessFailure::Http(envelope(TEAM_FAILURE))),
            BatchTeamLookup::Http(http) => return Err(AccessFailure::Http(http)),
        }
    } else {
        None
    };
    for model in models {
        match &team {
            Some(team) => check_team_model(access, caller, team, model).await?,
            None => check_key_model(access, caller, model).await?,
        }
    }
    Ok(())
}

// The team lookup runs only for an all-team-models key with a team, under a database that can answer
fn wants_team<Access>(
    access: &Access,
    caller: &UserAPIKeyAuth,
    fields: &BatchPrincipalFields,
) -> bool
where
    Access: BatchModelAccess,
{
    let lists_team = fields
        .models
        .as_deref()
        .is_some_and(|models| models.iter().any(|model| model.as_str() == ALL_TEAM_MODELS));
    lists_team && account_limits(caller).team_id.is_some() && access.database_available()
}

async fn check_team_model<Access>(
    access: &Access,
    caller: &UserAPIKeyAuth,
    team: &Access::Team,
    model: &str,
) -> Result<(), AccessFailure<Access::Error>>
where
    Access: BatchModelAccess,
{
    let check = access
        .team_can_access(model, caller, team)
        .await
        .map_err(AccessFailure::Source)?;
    match check {
        BatchAccessCheck::Allowed => {}
        BatchAccessCheck::Http(http) => return Err(AccessFailure::Http(http)),
        BatchAccessCheck::Failed { message } => {
            return Err(AccessFailure::Http(model_envelope(model, &message)));
        }
        // A team denial stands unless the key's own access group grants the model
        BatchAccessCheck::Denied { message } => {
            let grant = access
                .key_group_grants(model, caller, team)
                .await
                .map_err(AccessFailure::Source)?;
            match grant {
                BatchKeyGrant::Granted => {}
                BatchKeyGrant::Denied => {
                    return Err(AccessFailure::Http(model_envelope(model, &message)));
                }
                BatchKeyGrant::Failed { message: reason } => {
                    return Err(AccessFailure::Http(model_envelope(model, &reason)));
                }
                BatchKeyGrant::Http(http) => return Err(AccessFailure::Http(http)),
            }
        }
    }
    let member = access
        .member_can_access(model, caller, team)
        .await
        .map_err(AccessFailure::Source)?;
    access_outcome(member, model)
}

async fn check_key_model<Access>(
    access: &Access,
    caller: &UserAPIKeyAuth,
    model: &str,
) -> Result<(), AccessFailure<Access::Error>>
where
    Access: BatchModelAccess,
{
    let check = access
        .key_can_call(model, caller)
        .await
        .map_err(AccessFailure::Source)?;
    access_outcome(check, model)
}

// A denied or failed key/team-member check is the 403 envelope; an HTTPException passes through as it is
fn access_outcome<AccessError>(
    check: BatchAccessCheck,
    model: &str,
) -> Result<(), AccessFailure<AccessError>> {
    match check {
        BatchAccessCheck::Allowed => Ok(()),
        BatchAccessCheck::Http(http) => Err(AccessFailure::Http(http)),
        BatchAccessCheck::Denied { message } | BatchAccessCheck::Failed { message } => {
            Err(AccessFailure::Http(model_envelope(model, &message)))
        }
    }
}

// The team lookup failure carries no reason: LiteLLM replaces it with this fixed detail
const TEAM_FAILURE: &str =
    "Batch input file model access could not be validated against the current team.";

fn envelope(message: &str) -> BatchHttpError {
    let mut detail = JsonObject::new();
    detail.insert("error".to_owned(), Json::Text(message.to_owned()));
    BatchHttpError {
        status: 403,
        detail: Json::Object(detail),
    }
}

// The per-model rejection names the model and the auth helper's reason, as LiteLLM's f-string does
fn model_envelope(model: &str, reason: &str) -> BatchHttpError {
    envelope(&format!(
        "Batch input file references a model the caller is not authorized to use: model={model}, reason={reason}"
    ))
}
