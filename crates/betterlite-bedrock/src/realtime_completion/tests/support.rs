use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::realtime_completion::records::{CompletionEvent, DeltaChunk, DeltaType, ResponseState};

pub(super) const EVENT_1: &str = "event_00000000-0000-0000-0000-000000000001";
pub(super) const EVENT_2: &str = "event_00000000-0000-0000-0000-000000000002";
pub(super) const EVENT_3: &str = "event_00000000-0000-0000-0000-000000000003";
pub(super) const EVENT_4: &str = "event_00000000-0000-0000-0000-000000000004";
pub(super) const EVENT_5: &str = "event_00000000-0000-0000-0000-000000000005";
pub(super) const EVENT_6: &str = "event_00000000-0000-0000-0000-000000000006";
pub(super) const ITEM_DONE_BODY: &str = r#""output_index":0,"response_id":"resp_1","item":{"id":"item_1","object":"realtime.item","type":"message","status":"completed","role":"assistant","content":[]}}"#;

pub(super) struct FixedIds {
    values: Mutex<VecDeque<Uuid>>,
}

impl FixedIds {
    pub(super) fn sequence(count: u128) -> Self {
        Self {
            values: Mutex::new((1..=count).map(Uuid::from_u128).collect()),
        }
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or(Uuid::nil())
    }
}

pub(super) fn open_state(delta_type: Option<DeltaType>, chunks: Option<&[&str]>) -> ResponseState {
    let delta_chunks = chunks.map(|texts| {
        texts
            .iter()
            .map(|delta| DeltaChunk {
                delta: (*delta).to_owned(),
            })
            .collect()
    });
    ResponseState {
        output_item_id: Some("item_1".to_owned()),
        response_id: Some("resp_1".to_owned()),
        conversation_id: Some("conv_1".to_owned()),
        delta_type,
        delta_chunks,
    }
}

pub(super) fn wire(events: &[CompletionEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| serde_json::to_string(event).expect("an event serializes"))
        .collect()
}
