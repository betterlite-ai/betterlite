use super::records::{
    AudioDone, CompletionEvent, ContentPart, ContentPartDone, DeltaChunk, DeltaType, OutputItem,
    OutputItemDone, ResponseState, TextDone,
};
use crate::agentcore_stream::IdSource;

impl ResponseState {
    // The text chunks are cleared and the response identity stays for the next content block
    pub fn content_end_events(&mut self, ids: &dyn IdSource) -> Vec<CompletionEvent> {
        let (Some(item_id), Some(response_id)) =
            (self.output_item_id.as_deref(), self.response_id.as_deref())
        else {
            return Vec::new();
        };
        let mut events = part_done_events(
            self.delta_type,
            self.delta_chunks.as_deref(),
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
                content: [],
            },
        }));
        self.delta_chunks = None;
        events
    }
}

fn part_done_events(
    delta_type: Option<DeltaType>,
    chunks: Option<&[DeltaChunk]>,
    item_id: &str,
    response_id: &str,
    ids: &dyn IdSource,
) -> Vec<CompletionEvent> {
    let (done, part) = match delta_type {
        Some(DeltaType::Text) => {
            let text: String = chunks
                .unwrap_or_default()
                .iter()
                .map(|chunk| chunk.delta.as_str())
                .collect();
            let done = TextDone {
                content_index: 0,
                event_id: format!("event_{}", ids.next_id()),
                item_id: item_id.to_owned(),
                output_index: 0,
                response_id: response_id.to_owned(),
                text: text.clone(),
            };
            (CompletionEvent::TextDone(done), ContentPart::Text { text })
        }
        Some(DeltaType::Audio) => {
            let done = AudioDone {
                content_index: 0,
                event_id: format!("event_{}", ids.next_id()),
                item_id: item_id.to_owned(),
                output_index: 0,
                response_id: response_id.to_owned(),
            };
            let part = ContentPart::Audio {
                transcript: String::new(),
            };
            (CompletionEvent::AudioDone(done), part)
        }
        None => return Vec::new(),
    };
    let content_part = ContentPartDone {
        content_index: 0,
        event_id: format!("event_{}", ids.next_id()),
        item_id: item_id.to_owned(),
        output_index: 0,
        part,
        response_id: response_id.to_owned(),
    };
    vec![done, CompletionEvent::ContentPartDone(content_part)]
}
