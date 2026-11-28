use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::complexity_config::checks::Feature;
use betterlite_router::complexity_scoring::decision::RoutingDecision;
use betterlite_router::router_predicates::JsonMap;
use betterlite_router::tier_definitions::model::{TierPool, TierSet};

use crate::routing::cache_alternatives::config::CacheRoutingConfig;
use crate::routing::cache_alternatives::error::CacheAlternativesError;
use crate::routing::cache_alternatives::host::CacheRoutingHost;

const CLASSIFIED_CAUSES: [&str; 10] = [
    "heuristic_scorer",
    "heuristic_v2",
    "reasoning_override",
    "llm_classifier",
    "llm_v2_classifier",
    "jev_classifier",
    "capability_classifier",
    "heuristic_first_short_circuit",
    "hybrid_short_circuit",
    "classifier_plugin",
];
// routing policies a cache comparison must not bypass
const REFUSED_FEATURES: [Feature; 4] = [
    Feature::Adaptive,
    Feature::Plugins,
    Feature::SessionAffinity,
    Feature::UserTurnClassification,
];
const MARKER_MODEL: &str = "auto_router/complexity_router";
// _MARKER_OPTIONS: the only litellm_params keys a router marker may set
const MARKER_OPTIONS: [&str; 8] = [
    "model",
    "complexity_router_config",
    "rpm",
    "tpm",
    "tags",
    "timeout",
    "stream_timeout",
    "num_retries",
];
// LiteLLM_Params fields whose declared default is not None, so exclude_defaults drops an explicit false
const FALSE_DEFAULT_OPTIONS: [&str; 5] = [
    "allow_client_keepalive_override",
    "merge_reasoning_content_in_choices",
    "use_in_pass_through",
    "use_litellm_proxy",
    "use_xai_oauth",
];

// eligible_models: the (tier, model) pairs a cache comparison may weigh, deduplicated by model
#[must_use]
pub fn eligible_models(
    config: &CacheRoutingConfig<'_>,
    decision: &RoutingDecision,
) -> Vec<(String, String)> {
    let Some(tier) = decision_text(decision, "tier") else {
        return Vec::new();
    };
    let classified =
        decision_text(decision, "cause").is_some_and(|cause| CLASSIFIED_CAUSES.contains(&cause));
    if !classified || refused(config) {
        return Vec::new();
    }
    let order = config.tiers.tier_names();
    let Some(floor) = order.iter().position(|name| *name == tier) else {
        return Vec::new();
    };
    kept_tiers(config.tiers, order.get(floor..).unwrap_or_default())
}

// the eligible tiers in severity order, a model repeated across tiers keeping only its first tier
fn kept_tiers(tiers: &TierSet, order: &[&str]) -> Vec<(String, String)> {
    let mut kept: Vec<(String, String)> = Vec::new();
    for name in order {
        // a pool was refused before this runs, so only a single plain model name is left to keep
        let Some(TierPool::Single(entry)) = tiers.pools.get(*name) else {
            continue;
        };
        if kept
            .iter()
            .any(|(_, model)| model.as_str() == entry.model_name.as_str())
        {
            continue;
        }
        kept.push(((*name).to_owned(), entry.model_name.clone()));
    }
    kept
}

pub(crate) fn decision_text<'decision>(
    decision: &'decision RoutingDecision,
    field: &str,
) -> Option<&'decision str> {
    match decision.get(field) {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn refused(config: &CacheRoutingConfig<'_>) -> bool {
    if config.tiers.definitions.is_some() {
        return true;
    }
    if REFUSED_FEATURES
        .iter()
        .any(|feature| config.features.contains(feature))
    {
        return true;
    }
    config.tiers.pools.values().any(pool_refuses)
}

// a tier entry with litellm_params, or a list value (a pool), keeps the cache comparison out
fn pool_refuses(pool: &TierPool) -> bool {
    match pool {
        TierPool::Single(entry) => !entry.params.is_empty(),
        TierPool::Pool(_) => true,
    }
}

// supported_router_marker: every deployment behind the router alias is a bare auto_router marker
pub fn supported_router_marker<Host>(
    host: &Host,
    alias: &str,
    request_kwargs: &JsonMap,
) -> Result<bool, CacheAlternativesError>
where
    Host: CacheRoutingHost + ?Sized,
{
    let markers = host
        .deployments_for_request(alias, request_kwargs)
        .map_err(|source| CacheAlternativesError::Deployments {
            model: alias.to_owned(),
            source: Box::new(source),
        })?;
    Ok(!markers.is_empty()
        && markers
            .iter()
            .all(|marker| marker_params(&marker.litellm_params)))
}

fn marker_params(params: &JsonObject) -> bool {
    if !matches!(params.get("model"), Some(Json::Text(model)) if model.as_str() == MARKER_MODEL) {
        return false;
    }
    params
        .iter()
        .all(|(key, value)| marker_option(key.as_str(), value))
}

// model_dump(exclude_defaults=True, exclude_none=True) keeps a key only when it is not a marker option, null, or a false default
fn marker_option(key: &str, value: &Json) -> bool {
    MARKER_OPTIONS.contains(&key)
        || matches!(value, Json::Null)
        || (FALSE_DEFAULT_OPTIONS.contains(&key) && matches!(value, Json::Bool(false)))
}
