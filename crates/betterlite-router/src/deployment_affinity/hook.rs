use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::PreCallRequest;
use super::error::DeploymentAffinityError;
use super::json;
use crate::affinity_claims::identity::{
    AffinityFlag, marker_session_affinity_ttl, session_id_from_request_kwargs,
    user_key_from_request_kwargs,
};

// the identities async_pre_call_deployment_hook can claim for, and the ttl the session claim takes
pub(super) struct HookIdentities {
    pub(super) user_key: Option<String>,
    pub(super) caller_key: Option<String>,
    pub(super) session_id: Option<String>,
    pub(super) marker_ttl: Option<u32>,
}

// the marker ttl and the two identity reads, then LiteLLM's early returns when neither can claim
pub(super) fn hook_identities(
    flags: &BTreeSet<AffinityFlag>,
    kwargs: &JsonObject,
) -> Result<Option<HookIdentities>, DeploymentAffinityError> {
    let marker_ttl = marker_session_affinity_ttl(kwargs)?;
    let session_active = flags.contains(&AffinityFlag::Session) || marker_ttl.is_some();
    let user_key_enabled = flags.contains(&AffinityFlag::Deployment);
    if !user_key_enabled && !session_active {
        return Ok(None);
    }
    let user_key = user_key_from_request_kwargs(kwargs)?;
    let session_id = if session_active {
        session_id_from_request_kwargs(kwargs)?
    } else {
        None
    };
    let caller_key = user_key.clone().filter(|_| user_key_enabled);
    if caller_key.is_none() && session_id.is_none() {
        return Ok(None);
    }
    Ok(Some(HookIdentities {
        user_key,
        caller_key,
        session_id,
        marker_ttl,
    }))
}

// model_info's id, with LiteLLM's warning when the router attached no usable one
pub(super) fn selected_model_id(request: &PreCallRequest<'_>) -> Option<String> {
    let kwargs = request.request_kwargs;
    let Some(model_info) = request
        .model_info
        .or_else(|| json::request_model_info(kwargs))
        .or_else(|| json::metadata_model_info(kwargs))
    else {
        return None;
    };
    match model_info.get("id") {
        Some(Json::Text(model_id)) if !model_id.is_empty() => Some(model_id.clone()),
        None
        | Some(
            Json::Text(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        ) => {
            tracing::warn!(
                "DeploymentAffinityCheck: model_id missing; skipping affinity cache update."
            );
            None
        }
    }
}
