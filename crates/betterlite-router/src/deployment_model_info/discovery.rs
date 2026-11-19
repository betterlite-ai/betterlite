use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::compatible_models::TokenLimits;
use serde_json::Number;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::model_catalog_lookup::fields::text;

use super::cache::DiscoveryCache;

// The published catalog a read works against; the Arc's identity is what a cached discovery must still match.
pub struct CatalogSnapshot<'catalog> {
    pub catalog: &'catalog Arc<DeploymentCatalog>,
}

// Router.get_discovered_model_info: the advertised limits a deployment's configured model_info leaves absent or null.
#[must_use]
pub fn get_discovered_model_info(
    snapshot: &CatalogSnapshot<'_>,
    discovery: &DiscoveryCache,
    model_id: Option<&str>,
) -> JsonObject {
    let Some(id) = model_id else {
        return JsonObject::new();
    };
    let Some(record) = discovery.get(id) else {
        return JsonObject::new();
    };
    let Some(row) = snapshot.catalog.models.get(record.index) else {
        return JsonObject::new();
    };
    if !record.is_current(snapshot.catalog, id) {
        return JsonObject::new();
    }
    discovered_fields(record.limits, &row.model_info)
}

// The discovery map litellm returns: only the limits whose configured value is absent or null.
pub(crate) fn discovered_fields(limits: TokenLimits, configured: &JsonObject) -> JsonObject {
    let mut fields = JsonObject::new();
    for (key, limit) in [
        ("max_tokens", limits.max_tokens),
        ("max_input_tokens", limits.max_input_tokens),
        ("max_output_tokens", limits.max_output_tokens),
    ] {
        let Some(limit) = limit else {
            continue;
        };
        if matches!(configured.get(key), None | Some(Json::Null)) {
            fields.insert(key.to_owned(), Json::Number(Number::from(limit)));
        }
    }
    fields
}

// The configured non-null model_info over the discovery the deployment's id still holds, as every read composes it.
#[must_use]
pub fn merged_model_info(
    snapshot: &CatalogSnapshot<'_>,
    discovery: &DiscoveryCache,
    model_info: &JsonObject,
) -> JsonObject {
    let mut merged = get_discovered_model_info(snapshot, discovery, text(model_info, "id"));
    copy_configured(&mut merged, model_info);
    merged
}

// The fields model_info sets to anything but null, left in configured order.
pub fn copy_configured(merged: &mut JsonObject, model_info: &JsonObject) {
    for (key, value) in model_info {
        if !matches!(value, Json::Null) {
            merged.insert(key.clone(), value.clone());
        }
    }
}
