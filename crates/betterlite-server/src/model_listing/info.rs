use betterlite_auth::auth_model_lists::fallbacks::{
    CONTENT_POLICY_FALLBACK_TYPE, CONTEXT_WINDOW_FALLBACK_TYPE, FallbackGroups,
    GENERAL_FALLBACK_TYPE, get_all_fallbacks,
};
use betterlite_config::sections::fallback_plans::Settings;
use betterlite_models::call_helpers::token_limit::coerce_token_limit;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use serde_json::Number;

use crate::model_listing::ModelListingError;
use crate::model_listing::access::{DeploymentListingInfo, ListingRouter};

const MAX_INPUT_TOKENS: &str = "max_input_tokens";
const MAX_OUTPUT_TOKENS: &str = "max_output_tokens";
const MODE: &str = "mode";
const METADATA: &str = "metadata";
const FALLBACKS: &str = "fallbacks";
// LiteLLM's valid_fallback_types, in the order its 400 response lists them
const FALLBACK_TYPES: [&str; 3] = [
    GENERAL_FALLBACK_TYPE,
    CONTEXT_WINDOW_FALLBACK_TYPE,
    CONTENT_POLICY_FALLBACK_TYPE,
];

// create_model_info_response's arguments, with LiteLLM's defaults for the flag and the fallback type.
#[derive(Clone, Copy, Debug)]
pub struct ModelInfoRequest<'request> {
    pub model_id: &'request str,
    pub provider: &'request str,
    pub include_metadata: bool,
    pub fallback_type: Option<&'request str>,
}

// Everything create_model_info_response reads besides the request: the router, the cost map and the fallback plan.
pub struct ModelInfoSources<'sources> {
    pub router: Option<&'sources dyn ListingRouter>,
    pub cost_map: &'sources dyn ModelInfoSource,
    pub fallback_settings: Option<&'sources Settings>,
    pub fallback_groups: &'sources dyn FallbackGroups,
    // LiteLLM's DEFAULT_MODEL_CREATED_AT_TIME, which the anthropic_headers section reads from the env var of that name
    pub created_at: i64,
}

// LiteLLM's get_model_info: the cost map entry for a name, or the generalization rule answering instead of one.
pub trait ModelInfoSource: Send + Sync {
    fn model_info(&self, model: &str) -> Result<Option<JsonObject>, ModelInfoLookupError>;
    fn is_generalized(&self, info: &JsonObject) -> bool;
}

// A cost map that cannot answer for a name; LiteLLM's _safe_get_model_info catches whatever a lookup raises.
#[derive(Debug, thiserror::Error)]
#[error("the cost map cannot answer for {model:?}")]
pub struct ModelInfoLookupError {
    pub model: String,
}

pub fn create_model_info_response(
    request: &ModelInfoRequest<'_>,
    sources: &ModelInfoSources<'_>,
) -> Result<JsonObject, ModelListingError> {
    let alias_target = sources
        .router
        .and_then(|router| router.model_group_alias(request.model_id));
    let lookup_model = alias_target.as_deref().unwrap_or(request.model_id);
    let listing_info = sources
        .router
        .and_then(|router| router.listing_info(lookup_model));
    let cost_map_keys = listing_cost_map_keys(listing_info.as_ref());
    let listed_info = safe_model_info(sources.cost_map, lookup_model);
    let candidate_sets =
        listing_candidates(&cost_map_keys, lookup_model, listed_info.as_ref(), sources);
    let mut response = base_response(request, sources.created_at);
    insert_mode(&mut response, &candidate_sets, lookup_model, sources);
    let (max_input_tokens, max_output_tokens) =
        response_limits(&candidate_sets, listing_info.as_ref());
    insert_limit(&mut response, MAX_INPUT_TOKENS, max_input_tokens);
    insert_limit(&mut response, MAX_OUTPUT_TOKENS, max_output_tokens);
    if request.include_metadata {
        attach_metadata(&mut response, request, sources)?;
    }
    Ok(response)
}

// One entry per distinct model behind the listed name, and one nameless entry when the router knows none.
fn listing_cost_map_keys(listing_info: Option<&DeploymentListingInfo>) -> Vec<Option<&str>> {
    match listing_info {
        Some(info) if !info.cost_map_keys.is_empty() => info
            .cost_map_keys
            .iter()
            .map(|key| Some(key.as_str()))
            .collect(),
        Some(_) | None => vec![None],
    }
}

// One candidate list per distinct model behind the listed name, in the order the router gives them.
fn listing_candidates(
    cost_map_keys: &[Option<&str>],
    listed_model: &str,
    listed_info: Option<&JsonObject>,
    sources: &ModelInfoSources<'_>,
) -> Vec<Vec<JsonObject>> {
    cost_map_keys
        .iter()
        .map(|deployment_model| {
            resolve_listing_model_info(
                *deployment_model,
                listed_model,
                listed_info,
                sources.cost_map,
            )
        })
        .collect()
}

// the mode any candidate declares, then the one the router configures for the group
fn insert_mode(
    response: &mut JsonObject,
    candidate_sets: &[Vec<JsonObject>],
    lookup_model: &str,
    sources: &ModelInfoSources<'_>,
) {
    if let Some(mode) = first_mode(candidate_sets) {
        response.insert(MODE.to_owned(), Json::Text(mode));
    }
    if let Some(mode) = sources
        .router
        .and_then(|router| router.configured_mode(lookup_model))
    {
        response.insert(MODE.to_owned(), Json::Text(mode));
    }
}

// _group_token_limit over both fields, overridden by what the router's listing info declares.
fn response_limits(
    candidate_sets: &[Vec<JsonObject>],
    listing_info: Option<&DeploymentListingInfo>,
) -> (Option<i64>, Option<i64>) {
    let max_input = group_token_limit(candidate_sets, MAX_INPUT_TOKENS);
    let max_output = group_token_limit(candidate_sets, MAX_OUTPUT_TOKENS);
    let listed_input = listing_info.and_then(|info| info.max_input_tokens);
    let listed_output = listing_info.and_then(|info| info.max_output_tokens);
    (listed_input.or(max_input), listed_output.or(max_output))
}

// _safe_get_model_info: a cost map that cannot answer reads as no info, after one debug line.
fn safe_model_info(cost_map: &dyn ModelInfoSource, model: &str) -> Option<JsonObject> {
    match cost_map.model_info(model) {
        Ok(info) => info,
        Err(error) => {
            tracing::debug!(
                model,
                %error,
                "create_model_info_response: cost map lookup failed"
            );
            None
        }
    }
}

// _resolve_listing_model_info: both names are kept, the exact cost map entry ahead of a generalized one.
fn resolve_listing_model_info(
    deployment_model: Option<&str>,
    listed_model: &str,
    listed_info: Option<&JsonObject>,
    cost_map: &dyn ModelInfoSource,
) -> Vec<JsonObject> {
    let Some(deployment_model) = deployment_model.filter(|model| *model != listed_model) else {
        return listed_info.map_or_else(Vec::new, |info| vec![info.clone()]);
    };
    let Some(deployment_info) = safe_model_info(cost_map, deployment_model) else {
        return listed_info.map_or_else(Vec::new, |info| vec![info.clone()]);
    };
    let Some(listed_info) = listed_info else {
        return vec![deployment_info];
    };
    if cost_map.is_generalized(&deployment_info) && !cost_map.is_generalized(listed_info) {
        return vec![listed_info.clone(), deployment_info];
    }
    vec![deployment_info, listed_info.clone()]
}

// _first_token_limit: the first candidate that declares a usable limit for the field.
fn first_token_limit(candidates: &[JsonObject], field: &str) -> Option<i64> {
    candidates
        .iter()
        .find_map(|info| coerce_token_limit(info.get(field)))
}

// _group_token_limit: the widest limit any deployment behind the listed name declares, as /model_group/info reports it.
fn group_token_limit(candidate_sets: &[Vec<JsonObject>], field: &str) -> Option<i64> {
    candidate_sets
        .iter()
        .filter_map(|candidates| first_token_limit(candidates, field))
        .max()
}

// the first text mode any candidate declares, which a deployment's configured mode later overrides
fn first_mode(candidate_sets: &[Vec<JsonObject>]) -> Option<String> {
    candidate_sets
        .iter()
        .flatten()
        .find_map(|info| match info.get(MODE) {
            Some(Json::Text(mode)) => Some(mode.clone()),
            _ => None,
        })
}

// the OpenAI model object every entry starts from
fn base_response(request: &ModelInfoRequest<'_>, created_at: i64) -> JsonObject {
    JsonObject::from([
        ("id".to_owned(), Json::Text(request.model_id.to_owned())),
        ("object".to_owned(), Json::Text("model".to_owned())),
        ("created".to_owned(), Json::Number(Number::from(created_at))),
        (
            "owned_by".to_owned(),
            Json::Text(request.provider.to_owned()),
        ),
    ])
}

// a limit entry exists only where a declaration did
fn insert_limit(response: &mut JsonObject, field: &str, limit: Option<i64>) {
    if let Some(limit) = limit {
        response.insert(field.to_owned(), Json::Number(Number::from(limit)));
    }
}

// the metadata block: the fallbacks the router resolves for the model, which a listing never grants
fn attach_metadata(
    response: &mut JsonObject,
    request: &ModelInfoRequest<'_>,
    sources: &ModelInfoSources<'_>,
) -> Result<(), ModelListingError> {
    let fallback_type = request.fallback_type.unwrap_or(GENERAL_FALLBACK_TYPE);
    if !FALLBACK_TYPES.contains(&fallback_type) {
        return Err(ModelListingError::InvalidFallbackType);
    }
    let fallbacks = get_all_fallbacks(
        request.model_id,
        fallback_type,
        sources.fallback_settings,
        sources.fallback_groups,
    );
    let entries = Json::List(fallbacks.into_iter().map(Json::Text).collect());
    response.insert(
        METADATA.to_owned(),
        Json::Object(IndexMap::from([(FALLBACKS.to_owned(), entries)])),
    );
    Ok(())
}

pub fn validate_model_access(
    model_id: &str,
    available_models: &[String],
) -> Result<(), ModelListingError> {
    if model_id.contains(',') {
        let inaccessible: Vec<String> = model_id
            .split(',')
            .map(str::trim)
            .filter(|model| {
                !available_models
                    .iter()
                    .any(|available| available.as_str() == *model)
            })
            .map(str::to_owned)
            .collect();
        if !inaccessible.is_empty() {
            return Err(ModelListingError::BatchInaccessible {
                models: inaccessible,
            });
        }
        return Ok(());
    }
    if available_models
        .iter()
        .any(|available| available.as_str() == model_id)
    {
        return Ok(());
    }
    Err(ModelListingError::ModelInaccessible {
        model_id: model_id.to_owned(),
    })
}
