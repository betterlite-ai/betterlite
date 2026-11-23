pub mod error;
mod filter;
pub mod flags;
mod hook;
pub mod json;
mod read;
pub(crate) mod tests;

use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use self::error::DeploymentAffinityError;
use self::filter::{caller_selection, responses_deployment, session_deployment};
use self::hook::{HookIdentities, hook_identities, selected_model_id};
use crate::affinity_claims::AffinityClaims;
use crate::affinity_claims::error::AffinityError;
use crate::affinity_claims::identity::{
    AffinityFlag, AffinityKey, affinity_cache_key, effective_flags, marker_session_affinity_ttl,
    session_affinity_cache_key, stable_model_map_key, user_key_from_request_kwargs,
};
use crate::affinity_claims::pin::AffinityPin;
use crate::deployment_identity::spec::DeploymentSpec;

// Router.deployment_affinity_ttl_seconds's declaration default
pub const DEFAULT_TTL_SECONDS: u32 = 3600;
// DualCache.async_get_cache mirrors a shared hit into the local InMemoryCache under its own 600s default
const BACKFILL_TTL_SECONDS: u32 = 600;

// Router._is_priority_routing_group, injected so the routing group index keeps its alias resolution
pub type PriorityGroups = Arc<dyn Fn(&str) -> bool + Send + Sync>;

// DeploymentAffinityCheck.__init__: the global flags, the TTL and the per-group overrides
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub ttl_seconds: u32,
    pub defaults: BTreeSet<AffinityFlag>,
    pub model_group_affinity_config: Option<IndexMap<String, Vec<String>>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ttl_seconds: DEFAULT_TTL_SECONDS,
            defaults: BTreeSet::new(),
            model_group_affinity_config: None,
        }
    }
}

// async_filter_deployments' arguments: the healthy pool and the request arguments it reads
pub struct FilterRequest<'request> {
    pub model: &'request str,
    pub deployments: &'request [DeploymentSpec],
    pub request_kwargs: Option<&'request JsonObject>,
    pub target_order: Option<i64>,
}

// async_pre_call_deployment_hook's arguments: the selected deployment's model_info and the request metadata
pub struct PreCallRequest<'request> {
    pub request_kwargs: &'request JsonObject,
    pub model_info: Option<&'request JsonObject>,
}

// DeploymentAffinityCheck: caller, session and previous-response pins over one model group's healthy pool
pub struct DeploymentAffinity {
    claims: AffinityClaims,
    settings: Settings,
    is_priority_group: Option<PriorityGroups>,
}

impl DeploymentAffinity {
    #[must_use]
    pub fn new(
        claims: AffinityClaims,
        settings: Settings,
        is_priority_group: Option<PriorityGroups>,
    ) -> Self {
        Self {
            claims,
            settings,
            is_priority_group,
        }
    }

    // _get_effective_flags: an explicit group entry wins over the global flags
    #[must_use]
    pub fn flags_for(&self, model_group: &str) -> BTreeSet<AffinityFlag> {
        effective_flags(
            self.settings.model_group_affinity_config.as_ref(),
            model_group,
            &self.settings.defaults,
        )
    }

    // async_filter_deployments: previous response, then priority groups, then session, then caller
    pub async fn filter_deployments(
        &self,
        request: &FilterRequest<'_>,
    ) -> Result<Vec<DeploymentSpec>, DeploymentAffinityError> {
        if request.target_order.is_some() {
            return Ok(request.deployments.to_vec());
        }
        let flags = self.flags_for(request.model);
        let empty = JsonObject::new();
        let kwargs = request.request_kwargs.unwrap_or(&empty);
        if let Some(deployment) = responses_deployment(&flags, kwargs, request.deployments)? {
            return Ok(vec![deployment.clone()]);
        }
        if self
            .is_priority_group
            .as_ref()
            .is_some_and(|is_group| is_group(request.model))
        {
            return Ok(request.deployments.to_vec());
        }
        let Some(model_group) = stable_model_map_key(request.deployments) else {
            return Ok(request.deployments.to_vec());
        };
        let session = session_deployment(self, &flags, kwargs, request.deployments, &model_group);
        if let Some(deployment) = session.await? {
            return Ok(vec![deployment.clone()]);
        }
        caller_selection(self, &flags, kwargs, request.deployments, &model_group).await
    }

    // async_pre_call_deployment_hook: persist the selected deployment for the caller and the session
    pub async fn pre_call_deployment_hook(
        &self,
        request: &PreCallRequest<'_>,
    ) -> Result<(), DeploymentAffinityError> {
        let kwargs = request.request_kwargs;
        let Some(model_group) = json::deployment_model_name(kwargs) else {
            tracing::debug!(
                "DeploymentAffinityCheck: deployment_model_name missing in metadata; skipping affinity cache update."
            );
            return Ok(());
        };
        let flags = self.flags_for(model_group);
        let Some(identities) = hook_identities(&flags, kwargs)? else {
            return Ok(());
        };
        let Some(model_id) = selected_model_id(request) else {
            return Ok(());
        };
        let pin = AffinityPin(IndexMap::from([("model_id".to_owned(), model_id)]));
        self.claim_identities(model_group, identities, &pin).await;
        Ok(())
    }

    // _claim_pin for both identities: the caller under the router ttl, the session under the marker's
    async fn claim_identities(
        &self,
        model_group: &str,
        identities: HookIdentities,
        pin: &AffinityPin,
    ) {
        let session_ttl = identities.marker_ttl.unwrap_or(self.settings.ttl_seconds);
        if let Some(caller) = identities.caller_key {
            let key = affinity_cache_key(model_group, &caller);
            self.claim_pin(key, pin, self.settings.ttl_seconds).await;
        }
        if let Some(session) = identities.session_id {
            let key =
                session_affinity_cache_key(model_group, &session, identities.user_key.as_deref());
            self.claim_pin(key, pin, session_ttl).await;
        }
    }

    // _claim_pin: the first writer wins and a claim failure leaves affinity best effort
    async fn claim_pin(
        &self,
        key: Result<AffinityKey, AffinityError>,
        pin: &AffinityPin,
        ttl_seconds: u32,
    ) {
        let Ok(usable) = key else {
            tracing::debug!("deployment affinity: the cache key is unusable; skipping the claim");
            return;
        };
        // the box keeps the claim's async chain out of every caller's future layout
        let claimed = Box::pin(self.claims.claim(&usable, pin, ttl_seconds, None)).await;
        if let Err(error) = claimed {
            tracing::debug!(%error, "deployment affinity: the pin claim failed");
        }
    }
}

// the marker ttl, the effective flags and the caller identity both the filter and the hook read
struct AffinityInputs {
    marker_ttl: Option<u32>,
    session_active: bool,
    user_key_enabled: bool,
    user_key: Option<String>,
}

// _should_use_session_affinity's inputs, in LiteLLM's read order, including the scoped caller key
fn affinity_inputs(
    flags: &BTreeSet<AffinityFlag>,
    kwargs: &JsonObject,
) -> Result<AffinityInputs, DeploymentAffinityError> {
    let marker_ttl = marker_session_affinity_ttl(kwargs)?;
    let session_active = flags.contains(&AffinityFlag::Session) || marker_ttl.is_some();
    let user_key_enabled = flags.contains(&AffinityFlag::Deployment);
    let user_key = if session_active || user_key_enabled {
        user_key_from_request_kwargs(kwargs)?
    } else {
        None
    };
    Ok(AffinityInputs {
        marker_ttl,
        session_active,
        user_key_enabled,
        user_key,
    })
}
