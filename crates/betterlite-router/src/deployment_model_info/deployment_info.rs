use super::discovery::{CatalogSnapshot, get_discovered_model_info, holds};
use super::metadata::{ModelMetadataSource, RouterMetadata};
use super::text;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::model_registration::merge::update_dictionary;
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
pub fn get_model_group(
    lookup: &CatalogLookup<'_>,
    model_id: &str,
) -> Result<Option<Vec<DeploymentSpec>>, LookupError> {
    let Some(row) = lookup.get_deployment(model_id)? else {
        return Ok(None);
    };
    lookup.get_model_list(Some(&row.model_name), None).map(Some)
}
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
    let builtin = metadata
        .source
        .get_model_info(model_name, keys)
        .await
        .ok()
        .flatten();
    let builtin_key = builtin
        .as_ref()
        .and_then(|info| text(info, "key"))
        .map(str::to_owned);
    let base_key = base_defaults(metadata, &mut custom, keys).await;
    let mut info = combine(custom, builtin)?;
    let price_keys: Vec<&str> = [Some(model_id), builtin_key.as_deref(), base_key.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    if metadata.registry.cost_map_omits_token_price(&price_keys) {
        info.insert("input_cost_per_token".to_owned(), Json::Null);
        info.insert("output_cost_per_token".to_owned(), Json::Null);
    }
    Some(info)
}
fn custom_entry<Source>(
    metadata: &RouterMetadata<'_, '_, '_, Source>,
    model_id: &str,
) -> Option<JsonObject>
where
    Source: ModelMetadataSource,
{
    let raw = metadata.registry.model_cost.get(model_id);
    let discovered =
        get_discovered_model_info(metadata.snapshot, metadata.discovery, Some(model_id));
    match raw {
        None => None,
        Some(Json::Object(fields)) => {
            let mut custom = fields.clone();
            custom.extend(discovered);
            Some(custom)
        }
        Some(other) if holds(other) => None,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Some(discovered)
        }
    }
}
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
fn combine(custom: Option<JsonObject>, builtin: Option<JsonObject>) -> Option<JsonObject> {
    match (custom, builtin) {
        (Some(custom_fields), Some(mut builtin_info)) => {
            update_dictionary(&mut builtin_info, &custom_fields);
            Some(builtin_info)
        }
        (Some(custom_fields), None) => Some(custom_fields),
        (None, Some(builtin_info)) => Some(builtin_info),
        (None, None) => None,
    }
}
