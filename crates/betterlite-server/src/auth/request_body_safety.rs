use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, header};
use betterlite_auth::request_body_safety::{
    FormMetadataExtractor, RequestBodySafety, SafetyError, is_request_body_safe,
};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{MediaTypeKind, classify_media_type};
use betterlite_core::api::request_form::metadata::extract_nested_form_metadata;
use betterlite_core::api::request_form::values::{FormFields, FormValue, PartValue};
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::api::request_form::decode_request_form;
use crate::auth::auth_identity::{AuthContext, StageFailure};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;
use crate::state::request_body_safety::Slot;

mod tests;

#[derive(Clone, Debug)]
pub struct RequestBodySafetyStageInput {
    pub context: AuthContext,
    pub content_type: Option<String>,
    pub body: Bytes,
    pub slot: Option<Slot>,
}

pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    body: Option<&Bytes>,
) -> Result<RequestBodySafetyStageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let slot = request
        .extensions()
        .get::<Arc<ServerState>>()
        .map(|state| state.request_body_safety.clone());
    Ok(RequestBodySafetyStageInput {
        context,
        content_type,
        body: body.cloned().unwrap_or_default(),
        slot,
    })
}

pub(crate) fn run_stage(
    stage: &RequestBodySafetyStageInput,
) -> Result<AuthContext, Box<ProxyException>> {
    if let Some(slot) = &stage.slot {
        scan(slot, stage).map_err(|error| Box::new(error.into_proxy_exception()))?;
    }
    Ok(stage.context.clone())
}

// The auth body reader's dict; a body it cannot parse has nothing to scan, like LiteLLM.
fn scan(slot: &Slot, stage: &RequestBodySafetyStageInput) -> Result<(), SafetyError> {
    let content_type = stage.content_type.as_deref().unwrap_or_default();
    let Some(body) = request_object(&stage.body, content_type) else {
        return Ok(());
    };
    let model = request_model(&body);
    is_request_body_safe(&RequestBodySafety {
        body: &body,
        model,
        allow_client_side_credentials: slot.settings.allow_client_side_credentials,
        allowed_hosts: &slot.settings.allowed_hosts,
        grants: slot,
        metadata: &CoreMetadata,
        max_fallbacks: slot.settings.max_fallbacks,
    })
}

// request_data.get("model", ""): absent or non-text answers the empty name, which no group matches
pub(crate) fn request_model(body: &JsonObject) -> &str {
    match body.get("model") {
        Some(Json::Text(model)) => model.as_str(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "",
    }
}

pub(crate) fn request_object(bytes: &[u8], content_type: &str) -> Option<JsonObject> {
    match classify_media_type(content_type) {
        MediaTypeKind::Form => form_object(bytes, content_type),
        MediaTypeKind::Json | MediaTypeKind::Binary | MediaTypeKind::Other => json_object(bytes),
    }
}

fn json_object(bytes: &[u8]) -> Option<JsonObject> {
    let Ok(Json::Object(body)) = serde_json::from_slice::<Json>(bytes) else {
        return None;
    };
    Some(body)
}

// dict(request.form()): last value per key; an upload part keeps its key so the name is still checked.
fn form_object(bytes: &[u8], content_type: &str) -> Option<JsonObject> {
    let Ok(parts) = decode_request_form(bytes, content_type.as_bytes(), &ApiLimits::default())
    else {
        return None;
    };
    let mut body = JsonObject::new();
    for part in parts.items {
        let value = match part.value {
            PartValue::Text(text) => Json::Text(text),
            PartValue::Upload(_upload) => Json::Null,
        };
        body.insert(part.name, value);
    }
    Some(body)
}

// Only the extractor's top-level names matter: a URL-valued field is refused by name, not by value.
#[derive(Debug)]
pub(crate) struct CoreMetadata;

impl FormMetadataExtractor for CoreMetadata {
    fn top_level_names(&self, form_data: &JsonObject, prefix: &str) -> Vec<String> {
        let fields: FormFields = form_data
            .keys()
            .map(|name| (name.clone(), FormValue::Text(String::new())))
            .collect();
        let (tree, _warnings) = extract_nested_form_metadata(&fields, prefix);
        tree.keys().cloned().collect()
    }
}
