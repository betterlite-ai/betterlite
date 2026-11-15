use std::sync::{Mutex, PoisonError};

use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::realtime_output::NovaRealtimeOutput;
use crate::realtime_output::events::{ContentStart, OutputIds};
use crate::realtime_session::RealtimeSession;

pub(super) struct CountingIds {
    next: Mutex<u128>,
}

impl CountingIds {
    pub(super) const fn new() -> Self {
        Self {
            next: Mutex::new(1),
        }
    }

    pub(super) fn used(&self) -> u128 {
        let next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        next.saturating_sub(1)
    }
}

impl IdSource for CountingIds {
    fn next_id(&self) -> Uuid {
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        let id = Uuid::from_u128(*next);
        *next = next.saturating_add(1);
        id
    }
}

pub(super) fn tag(prefix: &str, number: u128) -> String {
    format!("{prefix}_{}", Uuid::from_u128(number))
}

pub(super) fn session() -> RealtimeSession {
    RealtimeSession::new(&CountingIds::new())
}

pub(super) fn projection<'state>(
    session: &'state mut RealtimeSession,
    ids: &'state CountingIds,
) -> NovaRealtimeOutput<'state> {
    NovaRealtimeOutput { session, ids }
}

pub(super) fn content_start(text: &str) -> ContentStart {
    serde_json::from_str(text).expect("a contentStart fixture")
}

pub(super) fn ids(response: Option<&str>, item: Option<&str>) -> OutputIds {
    OutputIds {
        response: response.map(str::to_owned),
        item: item.map(str::to_owned),
        conversation: None,
    }
}

pub(super) fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("a serializable event")
}
