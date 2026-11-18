use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;

use super::record::{Slot, Target, slot_target};
use crate::api::response_id_codecs::encrypted::wrap_encrypted_content;
use crate::api::response_id_traversal::metadata::{AffinityMetadata, ModelInfo};
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;

// only an added/done item the provider transform turned into a typed record carries cipher text here
pub fn wrap_cipher(event: &mut ResponsesAPIStreamingResponse, state: &ResponsesStreamBaseState) {
    if !enabled(state) {
        return;
    }
    let Some(model_id) = state
        .hidden_params
        .model_id
        .as_deref()
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    if let Some(Target::Extra(extra)) = slot_target(event, Slot::Item) {
        wrap_slot(extra.get_mut("encrypted_content"), model_id);
    }
}

pub fn request_metadata(state: &ResponsesStreamBaseState) -> AffinityMetadata {
    AffinityMetadata {
        model_info: state
            .hidden_params
            .model_id
            .clone()
            .map(|id| ModelInfo { id: Some(id) }),
        affinity_enabled: enabled(state),
    }
}

fn wrap_slot(slot: Option<&mut Json>, model_id: &str) {
    let Some(Json::Text(text)) = slot else {
        return;
    };
    if text.is_empty() {
        return;
    }
    *text = wrap_encrypted_content(text.as_str(), model_id);
}

fn enabled(state: &ResponsesStreamBaseState) -> bool {
    match state
        .litellm_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("encrypted_content_affinity_enabled"))
    {
        Some(Json::Bool(true)) => true,
        Some(
            Json::Null
            | Json::Bool(false)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => false,
    }
}
