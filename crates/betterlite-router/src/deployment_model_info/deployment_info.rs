use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::model_registration::merge::update_dictionary;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::fields::text;
use crate::model_catalog_lookup::lookup::CatalogLookup;

use super::discovery::{CatalogSnapshot, get_discovered_model_info};
use super::metadata::{ModelMetadataSource, RouterMetadata};
use super::value::holds;

// Router.get_model_info: the row the id index names, which a caller reads without copying.
pub fn get_model_info<'catalog>(
    snapshot: &CatalogSnapshot<'catalog>,
    model_id: &str,
) -> Result<Option<&'catalog DeploymentSpec>, LookupError> {
    let Some(index) = snapshot.catalog.id_index.get(model_id) else {
        return Ok(None);
    };
    match snapshot.catalog.models.get(*index) {
        Some(spec) => Ok(Some(spec)),
        None => Err(LookupError::InvalidRow { index: *index }),
    }
}

// Router.get_model_group: the same-name group of the deployment the id names, through B844's listing.
pub fn get_model_group(
    lookup: &CatalogLookup<'_>,
    model_id: &str,
) -> Result<Option<Vec<DeploymentSpec>>, LookupError> {
    let Some(row) = lookup.get_deployment(model_id)? else {
        return Ok(None);
    };
    lookup.get_model_list(Some(&row.model_name), None).map(Some)
}

// Router.get_deployment_model_info: the custom cost entry, the built-in entry and the custom entry's base model, each tried in turn.
pub async fn get_deployment_model_info<Source>(
    metadata: &RouterMetadata<'_, '_, '_, Source>,
    model_id: &str,
    model_name: &str,
    keys: &mut ModelCostKeys,
) -> Option<JsonObject>
where
    Source: ModelMetadataSource,
{
    let mut custom = custom_entry(metadata, model_id);
    let builtin = match metadata.source.get_model_info(model_name, keys).await {
        Ok(info) => info,
        Err(_failure) => None,
    };
    let builtin_key = builtin.as_ref().and_then(|info| text(info, "key"));
    let base_key = base_defaults(metadata, &mut custom, keys).await;
    let mut info = combine(custom, builtin)?;
    let price_keys: Vec<&str> = [Some(model_id), builtin_key, base_key.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    if metadata.registry.cost_map_omits_token_price(&price_keys) {
        info.insert("input_cost_per_token".to_owned(), Json::Null);
        info.insert("output_cost_per_token".to_owned(), Json::Null);
    }
    Some(info)
}

// litellm.model_cost.get(model_id) spread with the discovered limits, when the id is in the raw cost map at all.
fn custom_entry(
    metadata: &RouterMetadata<'_, '_, '_, impl ModelMetadataSource>,
    model_id: &str,
) -> Option<JsonObject> {
    match metadata.registry.model_cost.get(model_id) {
        None => None,
        Some(Json::Object(fields)) => {
            let mut custom = fields.clone();
            for (key, value) in
                get_discovered_model_info(metadata.snapshot, metadata.discovery, Some(model_id))
            {
                custom.insert(key, value);
            }
            Some(custom)
        }
        Some(other) if holds(other) => None,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => Some(
            get_discovered_model_info(metadata.snapshot, metadata.discovery, Some(model_id)),
        ),
    }
}

// The custom entry's non-null base_model, whose cost entry becomes the custom entry's defaults; failures are swallowed.
async fn base_defaults<Source>(
    metadata: &RouterMetadata<'_, '_, '_, Source>,
    custom: &mut Option<JsonObject>,
    keys: &mut ModelCostKeys,
) -> Option<String>
where
    Source: ModelMetadataSource,
{
    let custom_info = custom.as_ref()?;
    let Some(Json::Text(base_model)) = custom_info.get("base_model") else {
        return None;
    };
    let base_info = metadata
        .source
        .get_model_info(base_model, keys)
        .await
        .ok()
        .flatten()?;
    let base_key = text(&base_info, "key").map(str::to_owned);
    let mut merged = base_info;
    update_dictionary(&mut merged, custom.as_ref()?);
    *custom = Some(merged);
    base_key
}

// The three mutually exclusive sources: custom over built-in, built-in alone, custom alone, or none.
fn combine(custom: Option<JsonObject>, builtin: Option<JsonObject>) -> Option<JsonObject> {
    match (custom, builtin) {
        (Some(custom), Some(mut builtin)) => {
            update_dictionary(&mut builtin, &custom);
            Some(builtin)
        }
        (Some(custom), None) => Some(custom),
        (None, Some(builtin)) => Some(builtin),
        (None, None) => None,
    }
}
