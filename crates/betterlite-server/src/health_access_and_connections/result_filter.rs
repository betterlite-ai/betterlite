use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use crate::health_cycle::endpoints::endpoint_id;
use crate::health_probe_settings::display::ADMIN_ONLY_DISPLAY_PARAMS;

// _strip_admin_only_fields_from_health_result: a copy whose rows lose the provider routing fields, other keys untouched
#[must_use]
pub fn strip_admin_only_fields(result: &JsonObject) -> JsonObject {
    let mut output = result.clone();
    for key in ["healthy_endpoints", "unhealthy_endpoints"] {
        let Some(Json::List(endpoints)) = output.get(key).cloned() else {
            continue;
        };
        let stripped = endpoints
            .iter()
            .map(|endpoint| match endpoint {
                Json::Object(fields) => Json::Object(
                    fields
                        .iter()
                        .filter(|(name, _value)| {
                            !ADMIN_ONLY_DISPLAY_PARAMS.contains(&name.as_str())
                        })
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect(),
                ),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    endpoint.clone()
                }
            })
            .collect();
        output.insert(key.to_owned(), Json::List(stripped));
    }
    output
}

// _filter_health_check_results_by_model_ids: rows keep a shallow copy only when their non-empty model_id is allowed
#[must_use]
// ast-grep-ignore: no-long-names
pub fn filter_health_check_results_by_model_ids(
    results: &JsonObject,
    allowed_model_ids: &BTreeSet<String>,
) -> JsonObject {
    let healthy = retained_rows(results.get("healthy_endpoints"), allowed_model_ids);
    let unhealthy = retained_rows(results.get("unhealthy_endpoints"), allowed_model_ids);
    let healthy_count = Number::from(u64::try_from(healthy.len()).unwrap_or(u64::MAX));
    let unhealthy_count = Number::from(u64::try_from(unhealthy.len()).unwrap_or(u64::MAX));
    let mut filtered = JsonObject::new();
    filtered.insert(
        "healthy_endpoints".to_owned(),
        Json::List(healthy.into_iter().map(Json::Object).collect()),
    );
    filtered.insert(
        "unhealthy_endpoints".to_owned(),
        Json::List(unhealthy.into_iter().map(Json::Object).collect()),
    );
    filtered.insert("healthy_count".to_owned(), Json::Number(healthy_count));
    filtered.insert("unhealthy_count".to_owned(), Json::Number(unhealthy_count));
    filtered
}

// `(results.get(...) or [])` in LiteLLM: a missing or null list contributes nothing
fn retained_rows(
    endpoints: Option<&Json>,
    allowed_model_ids: &BTreeSet<String>,
) -> Vec<JsonObject> {
    let Some(Json::List(rows)) = endpoints else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| match row {
            Json::Object(fields) => endpoint_id(fields)
                .filter(|id| allowed_model_ids.contains(*id))
                .map(|_id| fields.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}
