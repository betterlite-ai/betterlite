use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};

use super::record::{Target, slot_target};
use crate::api::response_id_codecs::encrypted::wrap_encrypted_content;
use crate::api::response_id_traversal::metadata::{AffinityMetadata, ModelInfo};
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;

// only an added/done item the provider transform turned into a typed record carries cipher text here
pub fn wrap_cipher(event: &mut ResponsesAPIStreamingResponse, state: &ResponsesStreamBaseState) {
    let Some(model_id) = state
        .hidden_params
        .model_id
        .as_deref()
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    if !enabled(state) || !item_event(event) {
        return;
    }
    if let Some(Target::Extra(extra)) = slot_target(event) {
        wrap_slot(extra.get_mut("encrypted_content"), model_id);
    }
}

pub fn request_metadata(state: &ResponsesStreamBaseState) -> AffinityMetadata {
    let model_id = state.hidden_params.model_id.clone();
    AffinityMetadata {
        model_info: model_id.map(|id| ModelInfo { id: Some(id) }),
        affinity_enabled: enabled(state),
    }
}

fn item_event(event: &ResponsesAPIStreamingResponse) -> bool {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => matches!(
            known.as_ref(),
            KnownResponseEvent::OutputItemAdded(_) | KnownResponseEvent::OutputItemDone(_)
        ),
        ResponsesAPIStreamingResponse::Generic(record) => matches!(
            record.r#type.as_str(),
            "response.output_item.added" | "response.output_item.done"
        ),
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
    matches!(
        state
            .litellm_metadata
            .as_ref()
            .and_then(|metadata| metadata.get("encrypted_content_affinity_enabled")),
        Some(Json::Bool(true))
    )
}
