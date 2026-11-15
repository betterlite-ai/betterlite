use super::records::{
    CompletionEvent, ContentPart, ContentPartDone, DeltaChunk, DeltaType, OutputItem,
    OutputItemDone, PartDone, ResponseState,
};
use crate::agentcore_stream::IdSource;

// The text chunks are cleared and the response identity stays for the next content block
pub fn content_end_events(state: &mut ResponseState, ids: &dyn IdSource) -> Vec<CompletionEvent> {
    let (Some(item_id), Some(response_id)) = (
        state.output_item_id.as_deref(),
        state.response_id.as_deref(),
    ) else {
        return Vec::new();
    };
    let mut events = part_done_events(
        state.delta_type,
        state.delta_chunks.as_deref(),
        item_id,
        response_id,
        ids,
    );
    events.push(CompletionEvent::OutputItemDone(OutputItemDone {
        event_id: format!("event_{}", ids.next_id()),
        output_index: 0,
        response_id: response_id.to_owned(),
        item: OutputItem {
            id: item_id.to_owned(),
            object: "realtime.item",
            item_type: "message",
            status: "completed",
            role: "assistant",
            content: Vec::new(),
        },
    }));
    state.delta_chunks = None;
    events
}

fn part_done_events(
    delta_type: Option<DeltaType>,
    chunks: Option<&[DeltaChunk]>,
    item_id: &str,
    response_id: &str,
    ids: &dyn IdSource,
) -> Vec<CompletionEvent> {
    let Some(kind) = delta_type else {
        return Vec::new();
    };
    let (text, part) = part_content(kind, chunks);
    let done = PartDone {
        content_index: 0,
        event_id: format!("event_{}", ids.next_id()),
        item_id: item_id.to_owned(),
        output_index: 0,
        response_id: response_id.to_owned(),
        text,
    };
    let content_part = ContentPartDone {
        content_index: 0,
        event_id: format!("event_{}", ids.next_id()),
        item_id: item_id.to_owned(),
        output_index: 0,
        part,
        response_id: response_id.to_owned(),
    };
    let first = match kind {
        DeltaType::Text => CompletionEvent::TextDone(done),
        DeltaType::Audio => CompletionEvent::AudioDone(done),
    };
    vec![first, CompletionEvent::ContentPartDone(content_part)]
}

fn part_content(kind: DeltaType, chunks: Option<&[DeltaChunk]>) -> (Option<String>, ContentPart) {
    match kind {
        DeltaType::Text => {
            let joined: String = chunks
                .unwrap_or_default()
                .iter()
                .map(|chunk| chunk.delta.as_str())
                .collect();
            (Some(joined.clone()), ContentPart::Text { text: joined })
        }
        DeltaType::Audio => (
            None,
            ContentPart::Audio {
                transcript: String::new(),
            },
        ),
    }
}
