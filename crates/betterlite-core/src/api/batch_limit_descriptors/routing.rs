use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::BatchLimitDescriptors;
use crate::api::file_routing_ids::routing::decode_model_from_file_id;
use crate::api::managed_resource_ids::{is_base64_encoded_unified_id, target_model_names};
use crate::api::model_credentials::{
    CredentialLookupError, CredentialSource, get_credentials_for_model,
};
use crate::api::request_destination::RequestData;

// LitellM's operation_context for every batch input-file credential lookup
pub const OPERATION_CONTEXT: &str = "batch input file read (rate limiting)";

// LiteLLM's managed file id marker, which it matches without the routing prefix's colon
const MANAGED_FILE_PREFIX: &str = "litellm_proxy";

// A router failure that is not one of get_credentials_for_model's two HTTPException branches
#[derive(Debug, thiserror::Error)]
#[error("the credential source failed to resolve model `{model_id}`")]
pub struct BatchCredentialError<SourceError> {
    pub model_id: String,
    #[source]
    pub source: SourceError,
}

// The two branches LiteLLM catches are an absent answer here; anything else is a real failure
pub fn credentials_for<Source>(
    source: Option<&Source>,
    model_id: &str,
) -> Result<Option<JsonObject>, BatchCredentialError<Source::Error>>
where
    Source: CredentialSource,
{
    match get_credentials_for_model(source, model_id, OPERATION_CONTEXT) {
        Ok(credentials) => Ok(Some(credentials)),
        Err(CredentialLookupError::RouterMissing | CredentialLookupError::ModelNotFound(_)) => {
            Ok(None)
        }
        Err(CredentialLookupError::Source {
            model_id: failing,
            source: failing_source,
        }) => Err(BatchCredentialError {
            model_id: failing,
            source: failing_source,
        }),
    }
}

pub(super) fn text_member<'value>(data: &'value RequestData, name: &str) -> Option<&'value str> {
    match data.get(name) {
        Some(Json::Text(text)) if !text.is_empty() => Some(text.as_str()),
        Some(
            Json::Text(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

// The model bound to the input file id: a model-embedded file id first, then a unified managed file id
#[must_use]
pub fn file_bound_model(data: &RequestData) -> Option<String> {
    let input_file_id = text_member(data, "input_file_id")?;
    if let Some(model) = decode_model_from_file_id(input_file_id) {
        return Some(model);
    }
    let unified = is_base64_encoded_unified_id(input_file_id, MANAGED_FILE_PREFIX)?;
    target_model_names(&unified).into_iter().next()
}

// A file-bound model wins over the top-level one, as create_batch routes the batch
#[must_use]
pub fn batch_routing_model(data: &RequestData) -> Option<String> {
    if let Some(model) = file_bound_model(data) {
        return Some(model);
    }
    text_member(data, "model").map(str::to_owned)
}

// The provider comes from the deployment's trusted credentials, never from the request body
#[must_use]
pub fn provider_from(credentials: &JsonObject) -> Option<String> {
    match credentials.get("custom_llm_provider") {
        Some(Json::Text(provider)) if !provider.is_empty() => Some(provider.clone()),
        Some(
            Json::Text(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

pub(super) fn resolve_batch_provider<Limiter, Credentials>(
    service: &BatchLimitDescriptors<Limiter, Credentials>,
    model: Option<&str>,
) -> Result<Option<String>, BatchCredentialError<Credentials::Error>>
where
    Credentials: CredentialSource,
{
    let Some(bound) = model.filter(|name| !name.is_empty()) else {
        return Ok(None);
    };
    let Some(credentials) = credentials_for(service.credentials.as_ref(), bound)? else {
        return Ok(None);
    };
    Ok(provider_from(&credentials))
}
