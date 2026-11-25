use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_model_info::discovery::holds;

use super::{EnrichError, ModelInfoCatalog};

// All three get_litellm_model_info/get_model_info passes; the last two swallow every lookup failure and use {}.
pub(super) async fn lookup_catalog_info(
    model_info: &JsonObject,
    params: &JsonObject,
    catalog: &dyn ModelInfoCatalog,
) -> Result<JsonObject, EnrichError> {
    let mut found = first_pass(model_info, params, catalog).await;
    if found.is_empty() {
        found = second_pass(params, catalog).await;
    }
    if found.is_empty() {
        found = third_pass(params, catalog).await?;
    }
    Ok(found)
}

// get_litellm_model_info: base_model when the backend contains azure or the config names one, and nothing when that name is not text.
async fn first_pass(
    model_info: &JsonObject,
    params: &JsonObject,
    catalog: &dyn ModelInfoCatalog,
) -> JsonObject {
    let Some(Json::Text(backend)) = params.get("model") else {
        return JsonObject::new();
    };
    let target = if backend.contains("azure") || model_info.get("base_model").is_some_and(holds) {
        match model_info.get("base_model") {
            Some(Json::Text(base)) => Some(base.as_str()),
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => None,
        }
    } else {
        Some(backend.as_str())
    };
    let Some(target) = target else {
        return JsonObject::new();
    };
    catalog.model_info(target, None).await.unwrap_or_default()
}

async fn second_pass(params: &JsonObject, catalog: &dyn ModelInfoCatalog) -> JsonObject {
    let Some(Json::Text(backend)) = params.get("model") else {
        return JsonObject::new();
    };
    catalog.model_info(backend, None).await.unwrap_or_default()
}

// The value after the last slash, looked up with the value before the first slash as its provider.
async fn third_pass(
    params: &JsonObject,
    catalog: &dyn ModelInfoCatalog,
) -> Result<JsonObject, EnrichError> {
    let Some(value) = params.get("model") else {
        return Ok(JsonObject::new());
    };
    if !holds(value) {
        return Ok(JsonObject::new());
    }
    let Json::Text(backend) = value else {
        return Err(EnrichError::ModelNotText);
    };
    let provider = backend.split('/').next().unwrap_or_default();
    let model = backend.rsplit('/').next().unwrap_or_default();
    Ok(catalog
        .model_info(model, Some(provider))
        .await
        .unwrap_or_default())
}
