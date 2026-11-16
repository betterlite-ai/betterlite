use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_prompts::arguments::json::Json;

use super::error::ContentError;
use super::events::{
    ContentEvent, ContentPart, ContentPartDone, OutputItem, OutputItemDone, ResponseAudioDone,
    ResponseTextDone,
};
use super::ids::ContentIds;

pub struct DoneContent {
    pub event: ContentEvent,
    pub item_id: String,
    pub response_id: String,
    pub text: Option<String>,
}

pub fn content_done_event(
    delta_chunks: Option<&[Json]>,
    output_item_id: Option<&str>,
    response_id: Option<&str>,
    delta_type: AllDeltaType,
    ids: &mut dyn ContentIds,
) -> Result<DoneContent, ContentError> {
    let delta = joined_delta(delta_chunks)?;
    let item_id = output_item_id.map_or_else(|| format!("item_{}", ids.next_id()), str::to_owned);
    let response_id = response_id.map_or_else(|| format!("resp_{}", ids.next_id()), str::to_owned);
    let (event, text) = done_event(&item_id, &response_id, delta, delta_type, ids);
    Ok(DoneContent {
        event,
        item_id,
        response_id,
        text,
    })
}

fn joined_delta(delta_chunks: Option<&[Json]>) -> Result<String, ContentError> {
    let mut delta = String::new();
    for chunk in delta_chunks.unwrap_or_default() {
        let fields = chunk.as_object().ok_or(ContentError::DeltaChunkNotObject)?;
        let text = fields
            .get("delta")
            .and_then(Json::as_str)
            .ok_or(ContentError::MissingDeltaField)?;
        delta.push_str(text);
    }
    Ok(delta)
}

fn done_event(
    item_id: &str,
    response_id: &str,
    delta: String,
    delta_type: AllDeltaType,
    ids: &mut dyn ContentIds,
) -> (ContentEvent, Option<String>) {
    match delta_type {
        AllDeltaType::Text => (
            ContentEvent::TextDone(ResponseTextDone {
                content_index: 0,
                event_id: format!("event_{}", ids.next_id()),
                item_id: item_id.to_owned(),
                output_index: 0,
                response_id: response_id.to_owned(),
                text: delta.clone(),
            }),
            Some(delta),
        ),
        AllDeltaType::Audio => (
            ContentEvent::AudioDone(ResponseAudioDone {
                content_index: 0,
                event_id: format!("event_{}", ids.next_id()),
                item_id: item_id.to_owned(),
                output_index: 0,
                response_id: response_id.to_owned(),
            }),
            None,
        ),
    }
}

#[must_use]
pub fn additional_done_events(done: &DoneContent, ids: &mut dyn ContentIds) -> Vec<ContentEvent> {
    let part = done_part(done.text.as_deref());
    let item_id = done.item_id.clone();
    let response_id = done.response_id.clone();
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

fn done_part(text: Option<&str>) -> ContentPart {
    text.filter(|text| !text.is_empty()).map_or_else(
        || ContentPart::Audio {
            transcript: String::new(),
        },
        |text| ContentPart::Text {
            text: text.to_owned(),
        },
    )
}
