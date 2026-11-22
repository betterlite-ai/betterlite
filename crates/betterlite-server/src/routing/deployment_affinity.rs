use std::collections::BTreeSet;

use betterlite_config::sections::deployment_affinity::Settings as SectionSettings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::affinity_claims::AffinityClaims;
use betterlite_router::affinity_claims::identity::AffinityFlag;
use betterlite_router::deployment_affinity::flags::warn_unknown_affinity_flags;
use betterlite_router::deployment_affinity::{
    DeploymentAffinity, PreCallRequest, PriorityGroups, Settings,
};

mod tests;

// the router pieces the server hands the check: claims, the group index and whether a deployment stamps the marker
pub struct DeploymentAffinityInputs {
    pub claims: AffinityClaims,
    pub is_priority_group: Option<PriorityGroups>,
    pub session_marker_in_use: bool,
}

// add_optional_pre_call_checks and _ensure_deployment_affinity_callback: a global flag, a group entry or a marker-stamping router
#[must_use]
pub fn build(
    section: &SectionSettings,
    inputs: DeploymentAffinityInputs,
) -> Option<DeploymentAffinity> {
    let defaults = global_flags(section.optional_pre_call_checks.as_deref());
    for warning in warn_unknown_affinity_flags(section.model_group_affinity_config.as_ref()) {
        tracing::warn!("{}", warning.message());
    }
    let configured = section
        .model_group_affinity_config
        .as_ref()
        .is_some_and(|config| !config.is_empty());
    if defaults.is_empty() && !configured && !inputs.session_marker_in_use {
        return None;
    }
    Some(DeploymentAffinity::new(
        inputs.claims,
        Settings {
            ttl_seconds: section.ttl_seconds,
            defaults,
            model_group_affinity_config: section.model_group_affinity_config.clone(),
        },
        inputs.is_priority_group,
    ))
}

fn global_flags(checks: Option<&[String]>) -> BTreeSet<AffinityFlag> {
    let mut flags = BTreeSet::new();
    for check in checks.into_iter().flatten() {
        if let Some(flag) = AffinityFlag::from_name(check) {
            flags.insert(flag);
        }
    }
    flags
}

// the authenticated request fields async_pre_call_deployment_hook reads, as the auth stage left them
pub struct AuthenticatedRequest<'request> {
    pub metadata: Option<&'request Json>,
    pub litellm_metadata: Option<&'request Json>,
    pub model_info: Option<&'request Json>,
}

// async_pre_call_deployment_hook: record the deployment the router selected for this caller and session
pub async fn record_selection(affinity: &DeploymentAffinity, request: &AuthenticatedRequest<'_>) {
    let mut kwargs = JsonObject::new();
    for (channel, slot) in [
        ("litellm_metadata", request.litellm_metadata),
        ("metadata", request.metadata),
    ] {
        if let Some(Json::Object(fields)) = slot {
            kwargs.insert(channel.to_owned(), Json::Object(fields.clone()));
        }
    }
    let model_info = match request.model_info {
        Some(Json::Object(info)) => Some(info),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            None
        }
    };
    if let Err(error) = affinity
        .pre_call_deployment_hook(&PreCallRequest {
            request_kwargs: &kwargs,
            model_info,
        })
        .await
    {
        tracing::warn!(%error, "deployment affinity: the pre-call hook skipped an unusable request");
    }
}
