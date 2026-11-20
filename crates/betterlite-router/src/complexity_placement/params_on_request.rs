use betterlite_config::sections::complexity_placement as section;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::router_predicates::JsonMap;
use crate::router_predicates::params::{ParamSupport, as_output_cap};
use crate::routing_groups::params::tier_params_the_target_accepts;

use super::error::TierPlacementError;

const MAX_TOKENS: &str = "max_tokens";
const MAX_COMPLETION_TOKENS: &str = "max_completion_tokens";
const MAX_OUTPUT_TOKENS: &str = "max_output_tokens";
const CEILING: [&str; 3] = [MAX_TOKENS, MAX_COMPLETION_TOKENS, MAX_OUTPUT_TOKENS];
const CLIENT_CEILING: &str = "_client_output_ceiling";
const METADATA: &str = "metadata";
const LITELLM_METADATA: &str = "litellm_metadata";
const RAW_MODEL_NAME_KEY: &str = "_complexity_router_return_raw_model_name";
const REASONING_EFFORT: &str = "reasoning_effort";
const THINKING: &str = "thinking";
const EFFORT: &str = "effort";
const EFFORT_CARRIERS: [&str; 2] = ["output_config", "reasoning"];

// the model list and provider capabilities a tier's params are filtered against
pub struct TierTarget<'deps, S: ParamSupport> {
    pub lookup: &'deps CatalogLookup<'deps>,
    pub support: &'deps S,
}

// Router._pin_tier_params_onto_request: apply the accepted tier params and report whether they pinned a ceiling.
pub fn pin_tier_params_onto_request<S>(
    target: &TierTarget<'_, S>,
    model: &str,
    tier_params: &JsonMap,
    request_kwargs: &mut JsonMap,
    responses_call: bool,
) -> Result<bool, TierPlacementError>
where
    S: ParamSupport,
{
    if tier_params.is_empty() {
        return Ok(false);
    }
    let deployments = target.lookup.get_model_list(Some(model), None)?;
    let accepted = tier_params_the_target_accepts(
        model,
        tier_params,
        request_kwargs,
        &deployments,
        target.support,
    );
    let surface = ceiling_under_surface_name(accepted.params, responses_call);
    drop_client_carriers(request_kwargs, &surface);
    let pinned = surface.keys().any(|key| is_ceiling(key));
    for (key, value) in &surface {
        request_kwargs.insert(key.clone(), value.clone());
    }
    Ok(pinned)
}

// Router._restore_client_ceiling_no_tier_pins: when this pass pins no ceiling, hand back the caller's own carriers.
pub fn restore_client_ceiling_no_tier_pins(request_kwargs: &mut JsonMap) {
    let stamped =
        [METADATA, LITELLM_METADATA]
            .into_iter()
            .find_map(|key| match request_kwargs.get(key) {
                Some(Json::Object(bucket)) => bucket.get(CLIENT_CEILING).cloned(),
                Some(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
                )
                | None => None,
            });
    let Some(Json::Object(stamped)) = stamped else {
        return;
    };
    let mut callers = JsonMap::new();
    for carrier in CEILING {
        if let Some(value) = stamped.get(carrier)
            && let Some(cap) = as_output_cap(value)
        {
            callers.insert(carrier.to_owned(), Json::Number(Number::from(cap)));
        }
    }
    for carrier in CEILING {
        drop(request_kwargs.shift_remove(carrier));
    }
    for (carrier, value) in callers {
        request_kwargs.insert(carrier, value);
    }
}

// async_pre_routing_hook's first step: the response model field keeps the raw name this pass selected.
pub fn mark_raw_model_name(config: &section::Config, request_kwargs: &mut JsonMap) {
    if !config.return_raw_model_name {
        return;
    }
    let key = bucket_key(request_kwargs);
    if !request_kwargs.contains_key(key) {
        request_kwargs.insert(key.to_owned(), Json::Object(JsonMap::new()));
    }
    if let Some(Json::Object(metadata)) = request_kwargs.get_mut(key) {
        metadata.insert(RAW_MODEL_NAME_KEY.to_owned(), Json::Bool(true));
    }
}

fn bucket_key(request_kwargs: &JsonMap) -> &'static str {
    if request_kwargs.contains_key(LITELLM_METADATA) {
        LITELLM_METADATA
    } else {
        METADATA
    }
}

// Router._tier_ceiling_under_the_surface_name: collapse the three ceiling names onto the surface's own one.
fn ceiling_under_surface_name(tier_params: JsonMap, responses_call: bool) -> JsonMap {
    let surface = if responses_call {
        MAX_OUTPUT_TOKENS
    } else {
        MAX_TOKENS
    };
    let order: [&str; 3] = if responses_call {
        [MAX_OUTPUT_TOKENS, MAX_TOKENS, MAX_COMPLETION_TOKENS]
    } else {
        [MAX_TOKENS, MAX_COMPLETION_TOKENS, MAX_OUTPUT_TOKENS]
    };
    let Some(carried) = order.iter().find(|key| tier_params.contains_key(*key)) else {
        return tier_params;
    };
    let mut collapsed: JsonMap = tier_params
        .iter()
        .filter(|(key, _)| !is_ceiling(key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if let Some(value) = tier_params.get(*carried) {
        collapsed.insert(surface.to_owned(), value.clone());
    }
    collapsed
}

// Router._drop_client_carriers_a_tier_pin_supersedes: the request's own carrier of a pinned setting would outrank the tier's.
fn drop_client_carriers(request_kwargs: &mut JsonMap, tier_params: &JsonMap) {
    if tier_params.keys().any(|key| is_ceiling(key)) {
        stamp_client_ceiling(request_kwargs);
    }
    if !tier_params.contains_key(REASONING_EFFORT) {
        return;
    }
    drop(request_kwargs.shift_remove(THINKING));
    for carrier in EFFORT_CARRIERS {
        pop_effort_from_nested_carrier(request_kwargs, carrier);
    }
}

fn stamp_client_ceiling(request_kwargs: &mut JsonMap) {
    let key = bucket_key(request_kwargs);
    let mut stamped = JsonMap::new();
    for carrier in CEILING {
        if let Some(value) = request_kwargs.get(carrier) {
            stamped.insert(carrier.to_owned(), value.clone());
        }
    }
    if !matches!(request_kwargs.get(key), Some(Json::Object(_))) {
        request_kwargs.insert(key.to_owned(), Json::Object(JsonMap::new()));
    }
    if let Some(Json::Object(bucket)) = request_kwargs.get_mut(key) {
        bucket
            .entry(CLIENT_CEILING.to_owned())
            .or_insert_with(|| Json::Object(stamped));
    }
    for carrier in CEILING {
        drop(request_kwargs.shift_remove(carrier));
    }
}

fn pop_effort_from_nested_carrier(request_kwargs: &mut JsonMap, carrier: &str) {
    let Some(Json::Object(nested)) = request_kwargs.get(carrier).cloned() else {
        return;
    };
    let sanitized: JsonMap = nested
        .iter()
        .filter(|(key, _)| key.as_str() != EFFORT)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if sanitized.is_empty() {
        drop(request_kwargs.shift_remove(carrier));
    } else {
        request_kwargs.insert(carrier.to_owned(), Json::Object(sanitized));
    }
}

fn is_ceiling(key: &str) -> bool {
    CEILING.contains(&key)
}
