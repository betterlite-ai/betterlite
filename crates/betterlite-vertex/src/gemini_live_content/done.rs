use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_prompts::arguments::json::Json;

use super::error::ContentError;
use super::events::{
    ContentEvent, ContentPart, ContentPartDone, OutputItem, OutputItemDone, ResponseAudioDone,
    ResponseTextDone,
};
use super::ids::ContentIds;

pub fn content_done_event(
    delta_chunks: Option<&[Json]>,
    output_item_id: Option<&str>,
    response_id: Option<&str>,
    delta_type: AllDeltaType,
    ids: &mut dyn ContentIds,
) -> Result<ContentEvent, ContentError> {
    let mut delta = String::new();
    for chunk in delta_chunks.unwrap_or_default() {
        let fields = chunk.as_object().ok_or(ContentError::DeltaChunkNotObject)?;
        let text = fields
            .get("delta")
            .and_then(Json::as_str)
            .ok_or(ContentError::MissingDeltaField)?;
        delta.push_str(text);
    }
    let event_item_id =
        output_item_id.map_or_else(|| format!("item_{}", ids.next_id()), str::to_owned);
    let event_response_id =
        response_id.map_or_else(|| format!("resp_{}", ids.next_id()), str::to_owned);
    Ok(match delta_type {
        AllDeltaType::Text => ContentEvent::TextDone(ResponseTextDone {
            content_index: 0,
            event_id: format!("event_{}", ids.next_id()),
            item_id: event_item_id,
            output_index: 0,
            response_id: event_response_id,
            text: delta,
        }),
        AllDeltaType::Audio => ContentEvent::AudioDone(ResponseAudioDone {
            content_index: 0,
            event_id: format!("event_{}", ids.next_id()),
            item_id: event_item_id,
            output_index: 0,
            response_id: event_response_id,
        }),
    })
}

#[must_use]
pub fn additional_done_events(
    done: &ContentEvent,
    current_output_item_id: Option<&str>,
    current_response_id: Option<&str>,
    delta_type: AllDeltaType,
    ids: &mut dyn ContentIds,
) -> Vec<ContentEvent> {
    let text_part = match delta_type {
        AllDeltaType::Text => done.done_text().filter(|text| !text.is_empty()),
        AllDeltaType::Audio => None,
    };
    let part = text_part.map_or_else(
        || ContentPart::Audio {
            transcript: String::new(),
        },
        |text| ContentPart::Text {
            text: text.to_owned(),
        },
    );
    let item_id =
        current_output_item_id.map_or_else(|| format!("item_{}", ids.next_id()), str::to_owned);
    let response_id =
        current_response_id.map_or_else(|| format!("resp_{}", ids.next_id()), str::to_owned);
    vec![
        ContentEvent::ContentPartDone(ContentPartDone {
            content_index: 0,
            event_id: format!("event_{}", ids.next_id()),
            item_id: item_id.clone(),
            output_index: 0,
            part: part.clone(),
            response_id: response_id.clone(),
        }),
        ContentEvent::OutputItemDone(OutputItemDone {
            event_id: format!("event_{}", ids.next_id()),
            item: OutputItem {
                id: item_id,
                object: "realtime.item".to_owned(),
                item_type: "message".to_owned(),
                status: "completed".to_owned(),
                role: "assistant".to_owned(),
                content: vec![part],
            },
            output_index: 0,
            response_id,
        }),
    ]
}
