use std::sync::{Arc, Mutex, PoisonError};

use uuid::Uuid;

use crate::api::transcription_protocol::events::{EventIdSource, UuidEventIds};
use crate::api::vertex_chirp_protocol::transformer::{ItemIdSource, UuidItemIds};

const SESSION_ID_PREFIX: &str = "sess_";

pub trait ChirpIds: Send {
    fn event_id(&mut self) -> String;
    fn item_id(&mut self) -> String;
    fn session_id(&mut self) -> String;
}

#[derive(Debug, Default)]
pub struct UuidChirpIds {
    events: UuidEventIds,
    items: UuidItemIds,
}

impl ChirpIds for UuidChirpIds {
    fn event_id(&mut self) -> String {
        self.events.next_event_id()
    }

    fn item_id(&mut self) -> String {
        self.items.next_item_id()
    }

    fn session_id(&mut self) -> String {
        format!("{SESSION_ID_PREFIX}{}", Uuid::new_v4().simple())
    }
}

// the transformer and the adapter draw from one source, so a test's fixed sequence covers both
#[derive(Clone)]
pub(super) struct SharedIds(pub(super) Arc<Mutex<Box<dyn ChirpIds>>>);

impl SharedIds {
    pub(super) fn session_id(&self) -> String {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .session_id()
    }
}

impl EventIdSource for SharedIds {
    fn next_event_id(&mut self) -> String {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .event_id()
    }
}

impl ItemIdSource for SharedIds {
    fn next_item_id(&mut self) -> String {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .item_id()
    }
}
