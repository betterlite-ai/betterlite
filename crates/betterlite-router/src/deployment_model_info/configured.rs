use betterlite_models::call_helpers::token_limit::coerce_token_limit;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::lookup::CatalogLookup;

use super::cache::DiscoveryCache;
use super::discovery::{CatalogSnapshot, merged_model_info};

// Router.get_configured_token_limits: one B844-selected deployment's limits, discovery overlaid but never aggregated.
pub fn get_configured_token_limits(
    snapshot: &CatalogSnapshot<'_>,
    lookup: &CatalogLookup<'_>,
    discovery: &DiscoveryCache,
    model_name: &str,
) -> Result<(Option<i64>, Option<i64>), LookupError> {
    let Some(deployment) = lookup.deployment_by_group_name(model_name)? else {
        return Ok((None, None));
    };
    let model_info = merged_model_info(snapshot, discovery, &deployment.model_info);
    Ok((
        coerce_token_limit(model_info.get("max_input_tokens")),
        coerce_token_limit(model_info.get("max_output_tokens")),
    ))
}

// Router.get_configured_mode: the configured mode, a non-blank string returned untrimmed.
pub fn get_configured_mode(
    lookup: &CatalogLookup<'_>,
    model_name: &str,
) -> Result<Option<String>, LookupError> {
    let Some(deployment) = lookup.deployment_by_group_name(model_name)? else {
        return Ok(None);
    };
    Ok(configured_text(&deployment.model_info, "mode"))
}

// Router.get_configured_display_name: the configured display name, a non-blank string returned untrimmed.
pub fn get_configured_display_name(
    lookup: &CatalogLookup<'_>,
    model_name: &str,
) -> Result<Option<String>, LookupError> {
    let Some(deployment) = lookup.deployment_by_group_name(model_name)? else {
        return Ok(None);
    };
    Ok(configured_text(&deployment.model_info, "display_name"))
}

// isinstance(value, str) and value.strip(): the original text comes back, not the trimmed one.
fn configured_text(model_info: &JsonObject, field: &str) -> Option<String> {
    match model_info.get(field) {
        Some(Json::Text(value)) if !value.trim().is_empty() => Some(value.clone()),
        None
        | Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Text(_),
        ) => None,
    }
}
