use super::cache::DiscoveryCache;
use super::discovery::{CatalogSnapshot, holds, merged_model_info};
use super::types::DeploymentModelListingInfo;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::error::LookupError;
use betterlite_models::call_helpers::token_limit::coerce_token_limit;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
type ListedRow<'row> = (&'row DeploymentSpec, JsonObject);
pub fn get_model_listing_info(
    snapshot: &CatalogSnapshot<'_>,
    discovery: &DiscoveryCache,
    model_name: &str,
) -> Result<Option<DeploymentModelListingInfo>, LookupError> {
    let Some(indices) = snapshot.catalog.name_index.get(model_name) else {
        return Ok(None);
    };
    if indices.is_empty() {
        return Ok(None);
    }
    let mut rows: Vec<ListedRow<'_>> = Vec::with_capacity(indices.len());
    for index in indices {
        let Some(deployment) = snapshot.catalog.models.get(*index) else {
            return Err(LookupError::InvalidRow { index: *index });
        };
        let model_info = merged_model_info(snapshot, discovery, &deployment.model_info);
        rows.push((deployment, model_info));
    }
    Ok(Some(DeploymentModelListingInfo {
        cost_map_keys: cost_map_keys(&rows),
        max_input_tokens: widest_limit(&rows, "max_input_tokens"),
        max_output_tokens: widest_limit(&rows, "max_output_tokens"),
    }))
}
fn cost_map_keys(rows: &[ListedRow<'_>]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for (deployment, model_info) in rows {
        let params = &deployment.litellm_params;
        let chosen = [
            model_info.get("base_model"),
            params.get("base_model"),
            params.get("model"),
        ]
        .into_iter()
        .flatten()
        .find(|value| holds(value));
        let Some(Json::Text(key)) = chosen else {
            continue;
        };
        if keys.contains(key) {
            continue;
        }
        keys.push(key.clone());
    }
    keys
}
fn widest_limit(rows: &[ListedRow<'_>], field: &str) -> Option<i64> {
    rows.iter()
        .filter_map(|(_deployment, model_info)| coerce_token_limit(model_info.get(field)))
        .max()
}
