use betterlite_config::sections::request_metadata_safety::MissingSessionIdPolicy;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::metadata::MetadataField;

const SESSION_ID_OMITTED_KEY: &str = "litellm_session_id_omitted";
const SESSION_ID_KEY: &str = "session_id";
const BODY_SESSION_ID_KEY: &str = "litellm_session_id";
const METADATA_SLOTS: [MetadataField; 2] =
    [MetadataField::Metadata, MetadataField::LitellmMetadata];

// ast-grep-ignore: no-long-names
pub fn apply_missing_session_id_policy(
    data: &mut RequestData,
    field: MetadataField,
    policy: Option<&MissingSessionIdPolicy>,
    is_inference_route: bool,
) {
    // the omit stage of LiteLLM's policy; the generate, reject and unknown-policy stages belong to B1109
    for slot in METADATA_SLOTS {
        if let Some(Json::Object(metadata)) = data.get_mut(slot.as_str()) {
            metadata.shift_remove(SESSION_ID_OMITTED_KEY);
        }
    }
    let Some(resolved) = policy else {
        return;
    };
    if !is_inference_route || !matches!(data.get(field.as_str()), Some(Json::Object(_))) {
        return;
    }
    if !matches!(resolved, MissingSessionIdPolicy::Omit) {
        return;
    }
    let body_session_id = present_body_session_id(data);
    let requester_session_id = requester_session_id_present(data);
    let Some(Json::Object(metadata)) = data.get_mut(field.as_str()) else {
        return;
    };
    metadata.insert(SESSION_ID_OMITTED_KEY.to_owned(), Json::Bool(true));
    copy_body_session_id(metadata, body_session_id, requester_session_id);
}

// the body's session id is copied only when neither metadata bucket already supplies one
fn copy_body_session_id(
    metadata: &mut IndexMap<String, Json>,
    body_session_id: Option<Json>,
    requester_session_id: bool,
) {
    let Some(session_id) = body_session_id else {
        return;
    };
    if metadata
        .get(SESSION_ID_KEY)
        .is_some_and(|value| !matches!(value, Json::Null))
        || requester_session_id
    {
        return;
    }
    metadata.insert(SESSION_ID_KEY.to_owned(), session_id);
}

fn present_body_session_id(data: &RequestData) -> Option<Json> {
    match data.get(BODY_SESSION_ID_KEY) {
        Some(Json::Null) | None => None,
        Some(value) => Some(value.clone()),
    }
}

fn requester_session_id_present(data: &RequestData) -> bool {
    match data.get(MetadataField::Metadata.as_str()) {
        Some(Json::Object(requester)) => requester
            .get(SESSION_ID_KEY)
            .is_some_and(|value| !matches!(value, Json::Null)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => false,
    }
}
