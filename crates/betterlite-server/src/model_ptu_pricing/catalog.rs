use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::request_pricing::fields::COST_MAP_LOOKUP_KEY;
use betterlite_router::deployment_model_info::metadata::ModelMetadataSource;

use crate::model_effective_fields::decrypted_model;

// GetModelCostMap.loaded_model_cost_map: the catalog as loaded, before the router rewrites an entry with a deployment's overrides
pub trait LoadedCatalog {
    fn entry(&self, key: &str) -> Option<JsonObject>;
}

// a text base_model wins over the stored model, and a lookup that fails or finds nothing is an empty entry
#[must_use]
pub async fn cost_map_entry<Source>(
    source: &Source,
    keys: &mut ModelCostKeys,
    cipher: &DbCipher,
    stored_params: &JsonObject,
    incoming_model_info: &JsonObject,
) -> JsonObject
where
    Source: ModelMetadataSource,
{
    let lookup = match incoming_model_info.get("base_model") {
        Some(Json::Text(base_model)) => Some(base_model.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            decrypted_model(cipher, stored_params.get("model"))
        }
    };
    let Some(model) = lookup else {
        return JsonObject::new();
    };
    source
        .get_model_info(&model, keys)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

#[must_use]
pub fn loaded_catalog_entry(
    incoming_model_info: &JsonObject,
    catalog: &dyn LoadedCatalog,
) -> JsonObject {
    match incoming_model_info.get(COST_MAP_LOOKUP_KEY) {
        Some(Json::Text(key)) => catalog.entry(key).unwrap_or_default(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            JsonObject::new()
        }
    }
}
