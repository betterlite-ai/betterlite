use super::cache::DiscoveryCache;
use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::model_catalog_lookup::fields::text;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::compatible_models::TokenLimits;
use serde_json::Number;
use std::sync::Arc;
pub struct CatalogSnapshot<'catalog> {
    pub catalog: &'catalog Arc<DeploymentCatalog>,
}
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
pub(crate) fn discovered_fields(limits: TokenLimits, configured: &JsonObject) -> JsonObject {
    let found = [
        ("max_tokens", limits.max_tokens),
        ("max_input_tokens", limits.max_input_tokens),
        ("max_output_tokens", limits.max_output_tokens),
    ];
    let mut fields = JsonObject::new();
    for (key, limit) in found {
        let Some(found_limit) = limit else {
            continue;
        };
        if matches!(configured.get(key), None | Some(Json::Null)) {
            fields.insert(key.to_owned(), Json::Number(Number::from(found_limit)));
        }
    }
    fields
}
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
pub fn copy_configured(merged: &mut JsonObject, model_info: &JsonObject) {
    for (key, value) in model_info {
        if !matches!(value, Json::Null) {
            merged.insert(key.clone(), value.clone());
        }
    }
}
#[must_use]
pub fn holds(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number
            .as_f64()
            .is_none_or(|amount| amount > 0.0 || amount < 0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
