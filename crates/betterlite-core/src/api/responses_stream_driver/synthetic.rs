use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use indexmap::IndexMap;

use super::errors::DriverError;
use super::fallback::unclassified;
use super::live::Event;
// ast-grep-ignore: no-long-names
use crate::api::response_synthetic_events::builder::build_synthetic_response_events as build_events;
use crate::api::response_synthetic_events::error::SyntheticEventsError;
use crate::api::response_synthetic_events::stamping::ResponsesCostCalculator;
use crate::api::responses_stream_events::{
    BaseStreamStateInput, ResponsesStreamBaseState, base_stream_state,
};
use crate::api::responses_stream_hooks::ResponsesStreamHooks;
use crate::api::responses_stream_hooks::logging::log_completed_response;
use crate::api::responses_stream_hooks::ports::Dispatch;
use crate::api::responses_stream_hooks::state::ResponsesStreamHookState;

pub const CHUNK_SIZE: i64 = 5;

pub struct SyntheticSetup<'setup> {
    pub state: BaseStreamStateInput<'setup>,
    pub hooks: &'setup ResponsesStreamHooks<'setup>,
    pub cache_hit: Option<bool>,
    pub persist_before_logging: bool,
}

// one event per call keeps a caller's step boundary, as an iterator does
pub struct SyntheticStream<'stream> {
    hooks: &'stream ResponsesStreamHooks<'stream>,
    events: Vec<Event>,
    index: usize,
    state: ResponsesStreamBaseState,
    pub callback: ResponsesStreamHookState,
    details: IndexMap<String, Json>,
}

impl<'stream> SyntheticStream<'stream> {
    pub fn build<Calculator>(
        mut transformed: ResponsesApiResponse,
        setup: SyntheticSetup<'stream>,
        calculator: Option<&Calculator>,
    ) -> Result<Self, SyntheticEventsError>
    where
        Calculator: ResponsesCostCalculator,
    {
        let mut events = build_events(&mut transformed, calculator, CHUNK_SIZE)?;
        let mut state = base_stream_state(&setup.state);
        state.completed_response_cache_hit = setup.cache_hit;
        state.persist_before_logging = setup.persist_before_logging;
        state.completed_response = events.last().cloned();
        Ok(Self {
            hooks: setup.hooks,
            events,
            index: 0,
            state,
            callback: ResponsesStreamHookState::default(),
            details: IndexMap::new(),
        })
    }

    pub async fn advance(&mut self, dispatch: Dispatch) -> Result<Option<Event>, DriverError> {
        let Some(event) = self.events.get(self.index).cloned() else {
            return Ok(None);
        };
        self.index = self.index.saturating_add(1);
        if is_completed(&event) {
            self.state.completed_response = Some(event.clone());
            if let Err(failure) = log_completed_response(
                self.hooks,
                &mut self.state,
                &mut self.callback,
                &self.details,
                dispatch,
            )
            .await
            {
                return Err(DriverError::Class {
                    class: unclassified(failure.message),
                });
            }
        }
        Ok(Some(event))
    }
}

pub(super) fn is_completed(event: &Event) -> bool {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => {
            matches!(known.as_ref(), KnownResponseEvent::ResponseCompleted(_))
        }
        ResponsesAPIStreamingResponse::Generic(record) => record.r#type == "response.completed",
    }
}
