use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::as_object;
use super::records::SpendLogsRouterMetadata;

// _get_router_metadata_for_spend_log: only a deployment marked internal_router_model carries router provenance.
#[must_use]
pub fn router_metadata_for_spend_log(
    metadata: Option<&JsonObject>,
    requested_model: Option<&str>,
    selected_model: Option<&str>,
    selected_provider: Option<&str>,
    router_correlation_id: Option<&str>,
) -> Option<SpendLogsRouterMetadata> {
    let model_info = metadata
        .and_then(|metadata| metadata.get("model_info"))
        .and_then(as_object);
    let internal = model_info.and_then(|info| info.get("internal_router_model"));
    if !matches!(internal, Some(Json::Bool(true))) {
        return None;
    }
    Some(SpendLogsRouterMetadata {
        requested_model: nonempty(requested_model),
        selected_model: nonempty(selected_model),
        selected_provider: nonempty(selected_provider),
        router_correlation_id: router_correlation_id.map(ToOwned::to_owned),
    })
}

// `requested_model or None`: an empty string becomes null, while the correlation id passes through.
fn nonempty(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[path = "router_metadata/tests.rs"]
mod tests;
