use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::{
    PresidioGuardrailCallback, WsRequestDefaults,
};
use indexmap::IndexMap;
use serde_json::value::RawValue;

use super::request_masking::mask_create_frame;
use super::{FramePolicy, FramePolicyError, PiiCall, RequestData, decode_object};
use crate::api::response_id_codecs::response::decode_previous_response_id;
use crate::api::response_id_traversal::record::Record;
use crate::api::response_id_traversal::restore::restore_encrypted_item_ids;

// client keys win over fill_missing and overrides win over both; a nested response takes the merge itself
#[must_use]
pub fn with_request_defaults(
    defaults: Option<&WsRequestDefaults<Json>>,
    frame: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let Some(request_defaults) = defaults else {
        return frame.clone();
    };
    if let Some(Json::Object(nested)) = frame.get("response") {
        let merged = request_defaults.merged_into(&nested.clone().into_iter().collect());
        let mut applied = frame.clone();
        applied.insert(
            "response".to_owned(),
            Json::Object(merged.into_iter().collect()),
        );
        return applied;
    }
    let request: BTreeMap<String, Json> = frame.clone().into_iter().collect();
    let mut merged: IndexMap<String, Json> =
        request_defaults.merged_into(&request).into_iter().collect();
    if let Some(kind) = frame.get("type") {
        merged.insert("type".to_owned(), kind.clone());
    }
    merged
}

// overwriting the client's model stops a frame from switching to a deployment the connection wasn't authorized for
#[must_use]
pub fn enforce_authorized_model(
    authorized_model: Option<&str>,
    frame: &mut IndexMap<String, Json>,
) -> bool {
    let Some(model) = authorized_model.filter(|candidate| !candidate.is_empty()) else {
        return false;
    };
    let authorized = Json::Text(model.to_owned());
    let mut modified = false;
    if let Some(Json::Object(nested)) = frame.get_mut("response") {
        if nested.get("model") != Some(&authorized) {
            nested.insert("model".to_owned(), authorized.clone());
            modified = true;
        }
        if frame
            .get("model")
            .is_some_and(|current| current != &authorized)
        {
            frame.insert("model".to_owned(), authorized);
            modified = true;
        }
        return modified;
    }
    if frame.get("model") != Some(&authorized) {
        frame.insert("model".to_owned(), authorized);
        modified = true;
    }
    modified
}

#[must_use]
pub fn restore_wrapped_ids(frame: &IndexMap<String, Json>) -> Option<IndexMap<String, Json>> {
    let nested = match frame.get("response") {
        Some(Json::Object(fields)) => Some(fields),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            None
        }
    };
    let nested_fields = nested.map_or_else(IndexMap::new, restored_fields);
    let top_fields = restored_fields(frame);
    if nested_fields.is_empty() && top_fields.is_empty() {
        return None;
    }
    let mut restored = frame.clone();
    restored.extend(top_fields);
    let Some(container) = nested else {
        return Some(restored);
    };
    if nested_fields.is_empty() {
        return Some(restored);
    }
    let mut response = container.clone();
    response.extend(nested_fields);
    restored.insert("response".to_owned(), Json::Object(response));
    Some(restored)
}

// only the input list and previous_response_id are restored, and only a value that changed is kept
fn restored_fields(container: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut fields = IndexMap::new();
    if let Some(input @ Json::List(_)) = container.get("input") {
        let mut record = Record::from_json(input.clone());
        restore_encrypted_item_ids(&mut record);
        let restored = record.into_json();
        if &restored != input {
            fields.insert("input".to_owned(), restored);
        }
    }
    if let Some(Json::Text(previous)) = container.get("previous_response_id") {
        let original = decode_previous_response_id(previous);
        if &original != previous {
            fields.insert("previous_response_id".to_owned(), Json::Text(original));
        }
    }
    fields
}

pub async fn mask_response_create<'message, Guard>(
    policy: &FramePolicy<'_, Guard>,
    request_data: &mut RequestData,
    message: &'message str,
) -> Result<Cow<'message, str>, FramePolicyError<Guard::Error>>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Send + Sync,
    Guard::Error: 'static,
{
    let Some(parsed) = decode_object(message)? else {
        return Ok(Cow::Borrowed(message));
    };
    if !matches!(parsed.get("type"), Some(Json::Text(kind)) if kind == "response.create") {
        return Ok(Cow::Borrowed(message));
    }
    let mut authorized = with_request_defaults(policy.request_defaults, &parsed);
    let defaults_applied = authorized != parsed;
    let model_modified = enforce_authorized_model(policy.authorized_model, &mut authorized);
    let restored = restore_wrapped_ids(&authorized);
    let frame_modified = model_modified || restored.is_some() || defaults_applied;
    let mut frame = restored.unwrap_or(authorized);
    let masked = mask_with_guardrails(policy, request_data, &mut frame).await?;
    if frame_modified || masked {
        Ok(serde_json::to_string(&Json::Object(frame)).map(Cow::Owned)?)
    } else {
        Ok(Cow::Borrowed(message))
    }
}

// a callback that hands back the same text still counts as a change, as in LiteLLM
async fn mask_with_guardrails<Guard>(
    policy: &FramePolicy<'_, Guard>,
    request_data: &mut RequestData,
    frame: &mut IndexMap<String, Json>,
) -> Result<bool, FramePolicyError<Guard::Error>>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Send + Sync,
    Guard::Error: 'static,
{
    if policy.guardrail_callbacks.is_empty() {
        return Ok(false);
    }
    if let Entry::Vacant(slot) = request_data.entry("metadata".to_owned()) {
        slot.insert(RawValue::from_string("{}".to_owned())?);
    }
    let mut masked = false;
    for guardrail in policy.guardrail_callbacks {
        let settings = guardrail.callback.presidio_settings(request_data);
        let call = PiiCall {
            guard: &guardrail.callback,
            settings: settings.as_ref(),
            request_data,
            output_parse_pii: true,
        };
        let changed = mask_create_frame(&call, frame)
            .await
            .map_err(FramePolicyError::Guardrail)?;
        masked = changed || masked;
    }
    Ok(masked)
}
