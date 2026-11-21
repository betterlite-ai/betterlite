pub mod error;
pub mod flags;
pub mod json;
mod tests;

use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_core::api::response_id_codecs::response::get_model_id_from_response_id;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use self::error::DeploymentAffinityError;
use self::json::previous_response_id;
use crate::affinity_claims::AffinityClaims;
use crate::affinity_claims::error::AffinityError;
use crate::affinity_claims::identity::{
    AffinityFlag, AffinityKey, affinity_cache_key, effective_flags, find_deployment_by_model_id,
    marker_session_affinity_ttl, session_affinity_cache_key, session_id_from_request_kwargs,
    stable_model_map_key, user_key_from_request_kwargs,
};
use crate::affinity_claims::pin::{AffinityPin, StoredPin};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_value_store::{RouteValue, StoreRead};

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
        if flags.contains(&AffinityFlag::ResponsesApi)
            && let Some(previous) = previous_response_id(kwargs)?
            && let Some(model_id) = get_model_id_from_response_id(Some(previous))
            && let Some(deployment) = find_deployment_by_model_id(request.deployments, &model_id)
        {
            tracing::debug!(
                model_id,
                "DeploymentAffinityCheck: previous_response_id pinning"
            );
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
        let marker_ttl = marker_session_affinity_ttl(kwargs)?;
        let session_active = flags.contains(&AffinityFlag::Session) || marker_ttl.is_some();
        let user_key_enabled = flags.contains(&AffinityFlag::Deployment);
        let user_key = if session_active || user_key_enabled {
            user_key_from_request_kwargs(kwargs)?
        } else {
            None
        };
        if session_active
            && let Some(session_id) = session_id_from_request_kwargs(kwargs)?
            && let Ok(key) =
                session_affinity_cache_key(&model_group, &session_id, user_key.as_deref())
            && let Some(model_id) = self.pinned_model_id(&key).await
            && let Some(deployment) = find_deployment_by_model_id(request.deployments, &model_id)
        {
            tracing::debug!(
                model_id,
                session_id,
                "DeploymentAffinityCheck: session-id affinity hit"
            );
            return Ok(vec![deployment.clone()]);
        }
        if !user_key_enabled {
            return Ok(request.deployments.to_vec());
        }
        let Some(user_key) = user_key else {
            return Ok(request.deployments.to_vec());
        };
        let Ok(key) = affinity_cache_key(&model_group, &user_key) else {
            return Ok(request.deployments.to_vec());
        };
        let Some(model_id) = self.pinned_model_id(&key).await else {
            return Ok(request.deployments.to_vec());
        };
        let Some(deployment) = find_deployment_by_model_id(request.deployments, &model_id) else {
            tracing::debug!(
                model_id,
                "DeploymentAffinityCheck: pinned deployment not in healthy_deployments"
            );
            return Ok(request.deployments.to_vec());
        };
        tracing::debug!(model_id, "DeploymentAffinityCheck: caller affinity hit");
        Ok(vec![deployment.clone()])
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
        let marker_ttl = marker_session_affinity_ttl(kwargs)?;
        let session_active = flags.contains(&AffinityFlag::Session) || marker_ttl.is_some();
        let user_key_enabled = flags.contains(&AffinityFlag::Deployment);
        if !user_key_enabled && !session_active {
            return Ok(());
        }
        let user_key = user_key_from_request_kwargs(kwargs)?;
        let session_id = if session_active {
            session_id_from_request_kwargs(kwargs)?
        } else {
            None
        };
        if !(user_key_enabled && user_key.is_some()) && session_id.is_none() {
            return Ok(());
        }
        let model_info = request
            .model_info
            .or_else(|| json::metadata_model_info(kwargs));
        let Some(model_info) = model_info else {
            return Ok(());
        };
        let model_id = match model_info.get("id") {
            Some(Json::Text(model_id)) if !model_id.is_empty() => model_id,
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | Some(Json::Text(_)) => {
                tracing::warn!(
                    "DeploymentAffinityCheck: model_id missing; skipping affinity cache update."
                );
                return Ok(());
            }
        };
        let pin = AffinityPin(IndexMap::from([("model_id".to_owned(), model_id.clone())]));
        if user_key_enabled && let Some(user_key) = user_key.as_deref() {
            let key = affinity_cache_key(model_group, user_key);
            self.claim_pin(key, &pin, self.settings.ttl_seconds).await;
        }
        if let Some(session_id) = session_id.as_deref() {
            let ttl_seconds = marker_ttl.unwrap_or(self.settings.ttl_seconds);
            let key = session_affinity_cache_key(model_group, session_id, user_key.as_deref());
            self.claim_pin(key, &pin, ttl_seconds).await;
        }
        Ok(())
    }

    // DualCache.async_get_cache: the pod-local tier first, then the shared one, whose hit is mirrored back locally
    async fn pinned_model_id(&self, key: &AffinityKey) -> Option<String> {
        match self.claims.local.read(key) {
            Ok(Some(stored)) => {
                return stored
                    .pinned_model_id()
                    .filter(|model_id| !model_id.is_empty());
            }
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(%error, "deployment affinity: the pod-local pin is unreadable");
            }
        }
        let Some(store) = &self.claims.shared else {
            return None;
        };
        let observed = match store.get(&key.0).await {
            Ok(observed) => observed,
            Err(error) => {
                tracing::debug!(%error, "deployment affinity: the shared pin is unreadable");
                return None;
            }
        };
        let StoreRead::Value(RouteValue::Pin(value)) = observed else {
            return None;
        };
        let stored = StoredPin::Pin(AffinityPin(IndexMap::from([(
            "model_id".to_owned(),
            value.model_id,
        )])));
        if let Err(error) = self.claims.local.set(key, &stored, BACKFILL_TTL_SECONDS) {
            tracing::debug!(
                %error,
                "deployment affinity: the shared pin could not be mirrored locally"
            );
        }
        stored
            .pinned_model_id()
            .filter(|model_id| !model_id.is_empty())
    }

    // _claim_pin: the first writer wins and a claim failure leaves affinity best effort
    async fn claim_pin(
        &self,
        key: Result<AffinityKey, AffinityError>,
        pin: &AffinityPin,
        ttl_seconds: u32,
    ) {
        let Ok(key) = key else {
            tracing::debug!("deployment affinity: the cache key is unusable; skipping the claim");
            return;
        };
        if let Err(error) = self.claims.claim(&key, pin, ttl_seconds, None).await {
            tracing::debug!(%error, "deployment affinity: the pin claim failed");
        }
    }
}
