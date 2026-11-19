use super::cache::DiscoveryCache;
use super::discovery::{CatalogSnapshot, merged_model_info};
use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_models::call_helpers::token_limit::coerce_token_limit;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
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
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ConfiguredStrings {
    pub mode: Option<String>,
    pub display_name: Option<String>,
}
pub fn get_configured_strings(
    lookup: &CatalogLookup<'_>,
    model_name: &str,
) -> Result<ConfiguredStrings, LookupError> {
    let Some(deployment) = lookup.deployment_by_group_name(model_name)? else {
        return Ok(ConfiguredStrings::default());
    };
    Ok(ConfiguredStrings {
        mode: configured_text(&deployment.model_info, "mode"),
        display_name: configured_text(&deployment.model_info, "display_name"),
    })
}
fn configured_text(model_info: &JsonObject, field: &str) -> Option<String> {
    let Some(Json::Text(value)) = model_info.get(field) else {
        return None;
    };
    if value.trim().is_empty() {
        return None;
    }
    Some(value.clone())
}
