use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::{JsonObject, ModelRegistry};

use crate::api::proxy_endpoint_support::{RedactError, redact_deployment};
use crate::model_info_scope::text_field;

use self::catalog::lookup_catalog_info;
use self::merge::merge_model_info;

mod catalog;
mod merge;

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
