use super::{BACKFILL_TTL_SECONDS, DeploymentAffinity};
use crate::affinity_claims::identity::AffinityKey;
use crate::affinity_claims::pin::StoredPin;
use crate::routing_value_store::{RouteValue, StoreRead};

// DualCache.async_get_cache: the pod-local tier first, then the shared one, whose hit is mirrored back locally
pub(super) async fn pinned_model_id(
    service: &DeploymentAffinity,
    key: &AffinityKey,
) -> Option<String> {
    match service.claims.local.read(key) {
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
    let Some(store) = &service.claims.shared else {
        return None;
    };
    let observed = match store.get(&key.0).await {
        Ok(observed) => observed,
        Err(error) => {
            tracing::debug!(%error, "deployment affinity: the shared pin is unreadable");
            return None;
        }
    };
    if observed.warning.is_some() {
        tracing::debug!("deployment affinity: the shared pin read degraded");
        return None;
    }
    let StoreRead::Value(RouteValue::Pin(value)) = observed.value else {
        return None;
    };
    // the shared pin's payload is the serialized pin, as claim_affinity_pin writes it
    let stored = match StoredPin::parse(value.model_id.as_bytes()) {
        Ok(stored) => stored,
        Err(error) => {
            tracing::debug!(%error, "deployment affinity: the shared pin is not decodable");
            return None;
        }
    };
    if let Err(error) = service.claims.local.set(key, &stored, BACKFILL_TTL_SECONDS) {
        tracing::debug!(
            %error,
            "deployment affinity: the shared pin could not be mirrored locally"
        );
    }
    stored
        .pinned_model_id()
        .filter(|model_id| !model_id.is_empty())
}
