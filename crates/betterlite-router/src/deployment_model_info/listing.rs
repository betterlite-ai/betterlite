use betterlite_models::call_helpers::token_limit::coerce_token_limit;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::error::LookupError;

use super::cache::DiscoveryCache;
use super::discovery::{CatalogSnapshot, merged_model_info};
use super::types::DeploymentModelListingInfo;
use super::value::holds;

struct ListedRow<'catalog> {
    deployment: &'catalog DeploymentSpec,
    model_info: JsonObject,
}

// Router.get_model_listing_info: what the deployments behind an exact model name contribute to its /v1/models entry.
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
    let mut rows = Vec::with_capacity(indices.len());
    for index in indices {
        let Some(deployment) = snapshot.catalog.models.get(*index) else {
            return Err(LookupError::InvalidRow { index: *index });
        };
        rows.push(ListedRow {
            deployment,
            model_info: merged_model_info(snapshot, discovery, &deployment.model_info),
        });
    }
    Ok(Some(DeploymentModelListingInfo {
        cost_map_keys: cost_map_keys(&rows),
        max_input_tokens: widest_limit(&rows, "max_input_tokens"),
        max_output_tokens: widest_limit(&rows, "max_output_tokens"),
    }))
}

// Each row's truthy model_info.base_model, then params.base_model, then params.model, kept when nonempty text and deduplicated in config order.
fn cost_map_keys(rows: &[ListedRow<'_>]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for row in rows {
        let params = &row.deployment.litellm_params;
        let chosen = [
            row.model_info.get("base_model"),
            params.get("base_model"),
            params.get("model"),
        ]
        .into_iter()
        .flatten()
        .find(|value| holds(value));
        let Some(Json::Text(key)) = chosen else {
            continue;
        };
        if key.is_empty() || keys.contains(key) {
            continue;
        }
        keys.push(key.clone());
    }
    keys
}

// Router._widest_configured_limit: the largest usable value across the group, malformed values dropped by coercion.
fn widest_limit(rows: &[ListedRow<'_>], field: &str) -> Option<i64> {
    let limits: Vec<i64> = rows
        .iter()
        .filter_map(|row| coerce_token_limit(row.model_info.get(field)))
        .collect();
    limits.into_iter().max()
}
