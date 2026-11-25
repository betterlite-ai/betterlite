use betterlite_core::api::messages_response_stream::source::{ProviderEvent, ResponsesEvent};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

// One decoded Responses API event, in the mapping shape __anext__ reads with .get().
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub(super) struct EventSpec(IndexMap<String, Json>);

impl EventSpec {
    pub(super) fn event(&self) -> ResponsesEvent {
        ResponsesEvent {
            attributes: ProviderEvent::default(),
            mapping: Some(self.0.clone()),
        }
    }
}
