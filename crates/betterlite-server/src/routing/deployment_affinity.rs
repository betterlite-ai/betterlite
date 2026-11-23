use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_config::sections::deployment_affinity::Settings as SectionSettings;
use betterlite_config::types::Config;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_router::affinity_claims::AffinityClaims;
use betterlite_router::affinity_claims::identity::AffinityFlag;
use betterlite_router::affinity_claims::local::LocalPins;
use betterlite_router::deployment_affinity::flags::warn_unknown_affinity_flags;
use betterlite_router::deployment_affinity::{
    DeploymentAffinity, PreCallRequest, PriorityGroups, Settings,
};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::routing_value_store::StoreSettings;

use crate::cost_estimate_route::deployments::routing_groups;
use crate::routing::routing_value_store::{shared_store, wall_clock};

mod tests;

// the router pieces the server hands the check: claims, the group index and whether a deployment stamps the marker
pub struct DeploymentAffinityInputs {
    pub claims: AffinityClaims,
    pub is_priority_group: Option<PriorityGroups>,
    pub session_marker_in_use: bool,
}

// add_optional_pre_call_checks and _ensure_deployment_affinity_callback: a global flag, a group entry or a marker-stamping router
#[must_use]
pub fn configured(section: &SectionSettings, session_marker_in_use: bool) -> bool {
    !global_flags(section.optional_pre_call_checks.as_deref()).is_empty()
        || section
            .model_group_affinity_config
            .as_ref()
            .is_some_and(|config| !config.is_empty())
        || session_marker_in_use
}

#[must_use]
pub fn build(
    section: &SectionSettings,
    inputs: DeploymentAffinityInputs,
) -> Option<DeploymentAffinity> {
    let defaults = global_flags(section.optional_pre_call_checks.as_deref());
    for warning in warn_unknown_affinity_flags(section.model_group_affinity_config.as_ref()) {
        tracing::warn!("{}", warning.message());
    }
    if !configured(section, inputs.session_marker_in_use) {
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

// the check a router config registers, absent when the config turns affinity off or its caches cannot be built
#[must_use]
pub fn from_config(config: &Config) -> Option<DeploymentAffinity> {
    let section = &config.sections.deployment_affinity;
    let session_marker_in_use = config.sections.complexity_placement.stamps_session_marker();
    if !configured(section, session_marker_in_use) {
        return None;
    }
    build(
        section,
        DeploymentAffinityInputs {
            claims: claims(config)?,
            is_priority_group: priority_groups(config),
            session_marker_in_use,
        },
    )
}

// DualCache's two tiers: this worker's pins from the local ttl cache settings, the shared pins when Redis is configured
fn claims(config: &Config) -> Option<AffinityClaims> {
    let cached = &config.sections.local_ttl_cache;
    let store_settings = StoreSettings {
        max_size_in_memory: None,
        default_ttl_seconds: None,
        max_size_per_item: Some(1024),
        loaded_max_size_per_item_kib: cached.global_max_item_kib,
    };
    let local = match LocalPins::new(store_settings, wall_clock()) {
        Ok(local) => local,
        Err(error) => {
            tracing::error!(
                %error,
                "the deployment affinity in-memory cache settings are unreadable"
            );
            return None;
        }
    };
    Some(AffinityClaims {
        local,
        shared: shared_store(
            config,
            "the deployment affinity shared pin settings are unreadable",
            "the deployment affinity shared pin store cannot be built",
        ),
    })
}

// Router._is_priority_routing_group, resolved once from the routing groups and their aliases
pub(crate) fn priority_groups(config: &Config) -> Option<PriorityGroups> {
    let names: BTreeSet<String> = config
        .model_list
        .iter()
        .map(|deployment| deployment.model_name.clone())
        .collect();
    let index = match routing_groups(config, &names) {
        Ok(index) => Arc::new(index),
        Err(error) => {
            tracing::error!(
                %error,
                "the routing groups the deployment affinity check reads are unreadable"
            );
            return None;
        }
    };
    Some(Arc::new(move |model: &str| index.is_priority_group(model)))
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

// async_pre_call_deployment_hook's request arguments: the selected deployment stamped where the hook reads it
#[must_use]
pub fn stamped_kwargs(kwargs: Option<&Object>, deployment: &DeploymentSpec) -> JsonObject {
    let mut metadata = channel(kwargs, "metadata").unwrap_or_default();
    metadata.insert(
        "deployment_model_name".to_owned(),
        Json::Text(deployment.model_name.clone()),
    );
    metadata.insert(
        "model_info".to_owned(),
        Json::Object(deployment.model_info.clone()),
    );
    let mut stamped = JsonObject::new();
    if let Some(litellm_metadata) = channel(kwargs, "litellm_metadata") {
        stamped.insert(
            "litellm_metadata".to_owned(),
            Json::Object(litellm_metadata),
        );
    }
    stamped.insert("metadata".to_owned(), Json::Object(metadata));
    stamped
}

// async_pre_call_deployment_hook: record the deployment the router selected for this caller and session
pub async fn record_selection(
    affinity: &DeploymentAffinity,
    deployment: &DeploymentSpec,
    kwargs: Option<&Object>,
) {
    let request_kwargs = stamped_kwargs(kwargs, deployment);
    let request = PreCallRequest {
        request_kwargs: &request_kwargs,
        model_info: Some(&deployment.model_info),
    };
    if let Err(error) = affinity.pre_call_deployment_hook(&request).await {
        tracing::warn!(%error, "deployment affinity: the pre-call hook skipped an unusable request");
    }
}

// the request's own channel, converted to the router's JSON type; a value of another shape reads as absent
fn channel(kwargs: Option<&Object>, name: &str) -> Option<JsonObject> {
    let PromptJson::Object(fields) = kwargs?.get(name)? else {
        return None;
    };
    Some(
        fields
            .iter()
            .map(|(key, value)| (key.clone(), Json::from(value)))
            .collect(),
    )
}
