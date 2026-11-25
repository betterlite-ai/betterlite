use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::{JsonObject, ModelRegistry};
use betterlite_models::request_pricing::fields::{
    PRICING_OVERRIDES_KEY, is_server_derived_pricing_key, pricing_override_fields,
};
use betterlite_router::deployment_model_info::discovery::holds;

use crate::api::proxy_endpoint_support::{RedactError, redact_deployment};
use crate::model_info_scope::text_field;

const CREDENTIAL_NAME_KEY: &str = "litellm_credential_name";

#[derive(Debug, thiserror::Error)]
pub enum EnrichError {
    #[error("model_info is not an object")]
    ModelInfoNotObject,
    #[error("litellm_params.model is not a string")]
    ModelNotText,
    #[error(transparent)]
    Redact(#[from] RedactError),
}

#[async_trait]
pub trait ModelInfoCatalog: Send + Sync {
    async fn model_info(&self, model: &str, provider: Option<&str>) -> Option<JsonObject>;
}

pub trait RouterSource: Send + Sync {
    fn async_client_name(&self, model_id: Option<&str>) -> Option<String>;
    fn discovered_model_info(&self, model_id: Option<&str>) -> JsonObject;
}

pub struct EnrichContext<'a> {
    pub registry: &'a ModelRegistry,
    pub catalog: &'a dyn ModelInfoCatalog,
    pub router: Option<&'a dyn RouterSource>,
}

// _enrich_model_info_with_litellm_data: catalog passes, B157 pricing, B845 discovery, then B2400 redaction.
pub async fn enrich_model_info(
    model: &JsonObject,
    debug: bool,
    context: &EnrichContext<'_>,
) -> Result<JsonObject, EnrichError> {
    let empty_info = JsonObject::new();
    let model_info = info_object(model)?.unwrap_or(&empty_info);
    let empty_params = JsonObject::new();
    let params = match model.get("litellm_params") {
        Some(Json::Object(fields)) => fields,
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            &empty_params
        }
    };
    let mut enriched = model.clone();
    if debug {
        let client = debug_client_name(model_info, context.router);
        enriched.insert("openai_client".to_owned(), Json::Text(client));
    }
    let catalog_info = lookup_catalog_info(model_info, params, context.catalog).await?;
    let discovered = match context.router {
        Some(router) => router.discovered_model_info(text_field(model_info, "id")),
        None => JsonObject::new(),
    };
    let merged = merge_model_info(
        model_info,
        params,
        &catalog_info,
        &discovered,
        context.registry,
    );
    enriched.insert("model_info".to_owned(), Json::Object(merged));
    Ok(redact_deployment(
        enriched,
        &[CREDENTIAL_NAME_KEY.to_owned()],
    )?)
}

fn info_object(model: &JsonObject) -> Result<Option<&JsonObject>, EnrichError> {
    match model.get("model_info") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(EnrichError::ModelInfoNotObject)
        }
    }
}

fn debug_client_name(model_info: &JsonObject, router: Option<&dyn RouterSource>) -> String {
    let Some(router) = router else {
        return "llm_router_is_None".to_owned();
    };
    router
        .async_client_name(text_field(model_info, "id"))
        .unwrap_or_else(|| "None".to_owned())
}

// All three get_litellm_model_info/get_model_info passes; the last two swallow every lookup failure and use {}.
async fn lookup_catalog_info(
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
        text_field(model_info, "base_model")
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

// _pricing_override_stamps: the deployment's own server-derived prices, then the sorted field names they override.
fn pricing_override_stamps(model_info: &JsonObject, params: &JsonObject) -> JsonObject {
    let mut own = JsonObject::new();
    for (key, value) in params {
        if !matches!(value, Json::Null) && is_server_derived_pricing_key(key) {
            own.insert(key.clone(), value.clone());
        }
    }
    let mut stamps = own.clone();
    let overrides = pricing_override_fields(&[model_info, &own]);
    stamps.insert(
        PRICING_OVERRIDES_KEY.to_owned(),
        Json::List(overrides.into_iter().map(Json::Text).collect()),
    );
    stamps
}

// The merge keeps every stamped field, fills a stamped null only from discovery, and leaves inserted token prices null when unpriced.
fn merge_model_info(
    model_info: &JsonObject,
    params: &JsonObject,
    catalogue: &JsonObject,
    discovered: &JsonObject,
    registry: &ModelRegistry,
) -> JsonObject {
    let mut stamped = model_info.clone();
    for (key, value) in pricing_override_stamps(model_info, params) {
        stamped.insert(key, value);
    }
    let id = text_field(model_info, "id");
    let key = text_field(catalogue, "key");
    let keys: Vec<&str> = [id, key].into_iter().flatten().collect();
    let unpriced = registry.cost_map_omits_token_price(&keys);
    let mut combined = catalogue.clone();
    for (name, value) in discovered {
        combined.insert(name.clone(), value.clone());
    }
    for (name, value) in combined {
        let fill = match stamped.get(&name) {
            None => true,
            Some(Json::Null) => discovered.contains_key(&name),
            Some(
                Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_),
            ) => false,
        };
        if !fill {
            continue;
        }
        let value = if unpriced
            && matches!(
                name.as_str(),
                "input_cost_per_token" | "output_cost_per_token"
            ) {
            Json::Null
        } else {
            value
        };
        stamped.insert(name, value);
    }
    stamped
}
