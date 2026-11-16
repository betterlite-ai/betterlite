use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ChunkKind {
    Delta,
    Item,
}

impl ChunkKind {
    const fn event_type(self) -> &'static str {
        match self {
            Self::Delta => "response.output_text.delta",
            Self::Item => "response.output_item.done",
        }
    }
}

#[derive(Clone, Copy)]
pub enum ChunkSource<'event> {
    Single(&'event Json),
    List(&'event [Json]),
}

#[must_use]
pub fn update_current_chunks(
    message: ChunkSource<'_>,
    current: Option<Vec<Json>>,
    kind: ChunkKind,
) -> Option<Vec<Json>> {
    let wanted = kind.event_type();
    match message {
        ChunkSource::List(events) => {
            let chunks: Vec<Json> = events
                .iter()
                .filter(|event| is_event_type(event, wanted))
                .cloned()
                .collect();
            (!chunks.is_empty()).then_some(chunks)
        }
        ChunkSource::Single(event) => {
            if !is_event_type(event, wanted) {
                return None;
            }
            let mut chunks = current.unwrap_or_default();
            chunks.push(event.clone());
            Some(chunks)
        }
    }
}

fn is_event_type(event: &Json, wanted: &str) -> bool {
    event
        .as_object()
        .and_then(|fields| fields.get("type"))
        .and_then(Json::as_str)
        .is_some_and(|found| found == wanted)
}
