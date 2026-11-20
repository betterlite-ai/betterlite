use indexmap::IndexMap;
use sha2::{Digest as _, Sha256};

use betterlite_models::decisions::Json;

use crate::affinity_claims::identity::{
    AffinityKey, session_id_from_request_kwargs, user_key_from_request_kwargs,
};
use crate::affinity_claims::pin::{AffinityPin, StoredPin};
use crate::complexity_scoring::tier::Tier;
use crate::router_predicates::JsonMap;

use super::error::TierPlacementError;
use super::{RetainedPin, Selection, TierPlacement};

const MODEL_FIELD: &str = "model";
const CACHE_KEY_PREFIX: &str = "complexity_router_tier_model_affinity:v1:";
const ADAPTIVE_CHOSEN: &str = "adaptive_router_chosen_model";
const ADAPTIVE_DECISION: &str = "adaptive_router_decision";
const CHOSEN_MODEL: &str = "chosen_model";

// _pin_model_for_tier: a session's tier pin holds one model, and the claim's winner replaces a draw that lost.
pub async fn pin_model_for_tier(
    placement: &TierPlacement<'_>,
    tier: &Tier,
    selection: &Selection<'_>,
    request_kwargs: &mut JsonMap,
    retained_pin: Option<&RetainedPin>,
) -> Result<String, TierPlacementError> {
    let selected = selection.model;
    if !placement.deployment_pin || !contains(selection.candidates, selected) {
        return Ok(selected.to_owned());
    }
    let retained_model = retained_pin
        .filter(|pin| pin.tier.as_deref() == Some(tier.name()))
        .map(|pin| pin.model.as_str());
    if let Some(retained) = retained_model.filter(|model| contains(selection.candidates, *model)) {
        restamp_adaptive_choice(request_kwargs, selected, retained);
        return Ok(retained.to_owned());
    }
    let Some(session_id) = session_id_from_request_kwargs(request_kwargs)? else {
        return Ok(selected.to_owned());
    };
    let caller = user_key_from_request_kwargs(request_kwargs)?;
    let cache_key = tier_pin_cache_key(
        placement,
        tier,
        &session_id,
        caller.as_deref(),
        retained_model,
    )?;
    let requested = AffinityPin(model_pin(selected));
    let eligible: Vec<AffinityPin> = selection
        .candidates
        .iter()
        .map(|candidate| AffinityPin(model_pin(candidate)))
        .collect();
    let claimed = placement
        .claims
        .claim(
            &cache_key,
            &requested,
            placement.session_affinity_ttl_seconds,
            Some(&eligible),
        )
        .await?;
    let Some(pinned) = claimed_model(&claimed.pin) else {
        return Ok(selected.to_owned());
    };
    if !contains(selection.candidates, pinned) {
        return Ok(selected.to_owned());
    }
    restamp_adaptive_choice(request_kwargs, selected, pinned);
    Ok(pinned.to_owned())
}

// _restamp_adaptive_choice: the adaptive marker follows the model, so its rewards never land on the displaced one.
pub fn restamp_adaptive_choice(request_kwargs: &mut JsonMap, old_model: &str, new_model: &str) {
    let Some(Json::Object(metadata)) = request_kwargs.get_mut("metadata") else {
        return;
    };
    if matches!(metadata.get(ADAPTIVE_CHOSEN), Some(Json::Text(model)) if model == old_model) {
        metadata.insert(ADAPTIVE_CHOSEN.to_owned(), Json::Text(new_model.to_owned()));
    }
    if let Some(Json::Object(decision)) = metadata.get_mut(ADAPTIVE_DECISION)
        && matches!(decision.get(CHOSEN_MODEL), Some(Json::Text(model)) if model == old_model)
    {
        decision.insert(CHOSEN_MODEL.to_owned(), Json::Text(new_model.to_owned()));
    }
}

pub(crate) fn contains(candidates: &[String], model: &str) -> bool {
    candidates.iter().any(|candidate| candidate == model)
}

// _pin_model_for_tier's winner.get("model"): a dict's model field, never a bare stored string.
pub(crate) fn claimed_model(pin: &StoredPin) -> Option<&str> {
    match pin {
        StoredPin::Pin(fields) => fields.0.get(MODEL_FIELD).map(String::as_str),
        StoredPin::Other(Json::Object(fields)) => match fields.get(MODEL_FIELD) {
            Some(Json::Text(model)) => Some(model),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        },
        StoredPin::Text(_)
        | StoredPin::Other(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
        ) => None,
    }
}

fn model_pin(model: &str) -> IndexMap<String, String> {
    let mut fields = IndexMap::with_capacity(1);
    fields.insert(MODEL_FIELD.to_owned(), model.to_owned());
    fields
}

// _get_session_affinity_cache_key's tier-pin sibling: one key per router, config version, caller, session and tier.
#[must_use]
pub fn tier_pin_cache_key(
    placement: &TierPlacement<'_>,
    tier: &Tier,
    session_id: &str,
    caller: Option<&str>,
    retained_model: Option<&str>,
) -> Result<AffinityKey, TierPlacementError> {
    let model_name = placement.model_name;
    let affinity_config = placement.tier_affinity_config;
    let tier_name = tier.name();
    // json.dumps of LiteLLM's cache identity, so two identities can never collide on a delimiter
    let identity = match retained_model {
        None => {
            serde_json::to_string(&(model_name, affinity_config, caller, session_id, tier_name))
        }
        Some(model) => serde_json::to_string(&(
            model_name,
            affinity_config,
            caller,
            session_id,
            tier_name,
            ("replay_fallback", model),
        )),
    }
    .unwrap_or_default();
    let digest = Sha256::digest(identity.as_bytes());
    AffinityKey::try_from(format!("{CACHE_KEY_PREFIX}{}", hex::encode(digest)).as_str())
        .map_err(TierPlacementError::from)
}
