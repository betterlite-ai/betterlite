use std::mem;

use axum::body::{Body, Bytes, to_bytes};
use axum::http::Request;
use betterlite_auth::request_body_safety::{
    FormMetadataExtractor, RequestBodySafety, SafetyError, is_request_body_safe,
};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{MediaTypeKind, classify_media_type, parse_binary_body};
use betterlite_core::api::request_form::metadata::extract_nested_form_metadata;
use betterlite_core::api::request_form::values::{FormFields, FormValue, PartValue};
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::api::request_form::decode_request_form;
use crate::auth::auth_identity::AuthContext;
use crate::state::request_body_safety::Slot;

mod tests;

#[derive(Clone, Debug)]
pub struct RequestBodySafetyStageInput {
    pub context: AuthContext,
    pub content_type: Option<String>,
    pub body: Bytes,
    pub slot: Option<Slot>,
}

// The body is read once, before this stage, and handed back unchanged to the handler it was taken from.
pub(crate) async fn buffer_body(request: &mut Request<Body>) -> Result<Bytes, axum::Error> {
    let body = mem::replace(request.body_mut(), Body::empty());
    to_bytes(body, usize::MAX).await
}

pub(crate) fn run_stage(
    stage: &RequestBodySafetyStageInput,
) -> Result<AuthContext, Box<ProxyException>> {
    if let Some(slot) = &stage.slot {
        scan(slot, stage).map_err(|error| Box::new(error.into_proxy_exception()))?;
    }
    Ok(stage.context.clone())
}

// LiteLLM hands is_request_body_safe the dict its auth-time body reader parsed; the buffered body is read
// the same way here, and a body the reader cannot parse has nothing to scan.
fn scan(slot: &Slot, stage: &RequestBodySafetyStageInput) -> Result<(), SafetyError> {
    let content_type = stage.content_type.as_deref().unwrap_or_default();
    let Some(body) = request_object(&stage.body, content_type) else {
        return Ok(());
    };
    // request_data.get("model", ""): absent or non-text answers the empty name, which no group matches
    let model = match body.get("model") {
        Some(Json::Text(model)) => model.as_str(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "",
    };
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

fn request_object(bytes: &[u8], content_type: &str) -> Option<JsonObject> {
    match classify_media_type(content_type) {
        MediaTypeKind::Form => form_object(bytes, content_type),
        // _parse_binary_body: JSON sent under a binary content type still parses, real binary carries no params
        MediaTypeKind::Binary => Some(parse_binary_body(bytes)),
        MediaTypeKind::Json | MediaTypeKind::Other => json_object(bytes),
    }
}

fn json_object(bytes: &[u8]) -> Option<JsonObject> {
    let Ok(Json::Object(body)) = serde_json::from_slice::<Json>(bytes) else {
        return None;
    };
    Some(body)
}

// dict(request.form()): the last value per exact key, and an upload part keeps its key because the banned
// check only asks whether the name is present.
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

// B1101 builds the bracket-form tree; its top-level names are all the policy needs, because every
// URL-valued banned field is refused by name before a value could admit it.
#[derive(Debug)]
struct CoreMetadata;

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
