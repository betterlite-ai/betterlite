use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use indexmap::IndexMap;

use super::errors::DriverError;
use super::live::Event;
use crate::api::response_synthetic_events::builder::build_synthetic_response_events;
use crate::api::response_synthetic_events::error::SyntheticEventsError;
use crate::api::response_synthetic_events::stamping::ResponsesCostCalculator;
use crate::api::responses_stream_events::{
    BaseStreamStateInput, LoggingSnapshot, ResponsesApiBase, ResponsesStreamBaseState, StreamTimes,
    base_stream_state,
};
use crate::api::responses_stream_hooks::ResponsesStreamHooks;
use crate::api::responses_stream_hooks::failure::await_pending_logging;
use crate::api::responses_stream_hooks::logging::log_completed_response;
use crate::api::responses_stream_hooks::ports::Dispatch;
use crate::api::responses_stream_hooks::state::ResponsesStreamHookState;

pub const CHUNK_SIZE: i64 = 5;

pub struct SyntheticSetup<'setup> {
    pub hooks: &'setup ResponsesStreamHooks<'setup>,
    pub model: &'setup str,
    pub custom_llm_provider: Option<&'setup str>,
    pub litellm_metadata: Option<&'setup IndexMap<String, Json>>,
    pub request_data: Option<&'setup IndexMap<String, Json>>,
    pub call_type: Option<&'setup str>,
    pub headers: &'setup IndexMap<String, String>,
    pub api_base: &'setup dyn ResponsesApiBase,
    pub logging: &'setup StreamLoggingObject,
    pub times: StreamTimes,
    pub cache_hit: Option<bool>,
    pub persist_before_logging: bool,
}

/// B928's event list for a whole response, handed out one event per call.
pub struct SyntheticStream<'stream> {
    hooks: &'stream ResponsesStreamHooks<'stream>,
    events: Vec<Event>,
    index: usize,
    state: ResponsesStreamBaseState,
    callback: ResponsesStreamHookState,
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
        let mut events = build_synthetic_response_events(&mut transformed, calculator, CHUNK_SIZE)?;
        let mut state = {
            let input = BaseStreamStateInput {
                model: setup.model,
                custom_llm_provider: setup.custom_llm_provider,
                litellm_metadata: setup.litellm_metadata,
                request_data: setup.request_data,
                call_type: setup.call_type,
                logging: LoggingSnapshot::read(setup.logging),
                headers: setup.headers,
                api_base: setup.api_base,
                times: setup.times,
                max_streaming_duration_seconds: None,
            };
            base_stream_state(&input)
        };
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

    pub fn next_event(&mut self) -> Result<Option<Event>, DriverError> {
        futures_executor::block_on(self.advance(Dispatch::Inline))
    }

    pub async fn next_event_async(&mut self) -> Result<Option<Event>, DriverError> {
        self.advance(Dispatch::Spawned).await
    }

    pub async fn drain_logging(&mut self) {
        await_pending_logging(&mut self.callback).await;
    }

    async fn advance(&mut self, dispatch: Dispatch) -> Result<Option<Event>, DriverError> {
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
                    class: ErrorClass::Unclassified(Box::new(ErrorContext {
                        message: failure.message,
                        model: None,
                        provider: None,
                        max_retries: None,
                        num_retries: None,
                        body_type: None,
                        body_param: None,
                        body_code: None,
                    })),
                });
            }
        }
        Ok(Some(event))
    }
}

fn is_completed(event: &Event) -> bool {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => {
            matches!(known.as_ref(), KnownResponseEvent::ResponseCompleted(_))
        }
        ResponsesAPIStreamingResponse::Generic(record) => record.r#type == "response.completed",
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, is_completed};

    fn decode(text: &str) -> Event {
        serde_json::from_str(text).expect("the fixture event decodes")
    }

    #[test]
    fn a_known_completed_event_is_terminal() {
        assert!(is_completed(&decode(
            r#"{"type":"response.completed","response":{"id":"resp_1","created_at":1,"object":"response","output":[]}}"#
        )));
    }

    #[test]
    fn a_generic_completed_event_is_terminal() {
        assert!(is_completed(&decode(
            r#"{"type":"response.completed","response":"not a response body"}"#
        )));
    }

    #[test]
    fn a_created_event_is_not_terminal() {
        assert!(!is_completed(&decode(
            r#"{"type":"response.created","response":{"id":"resp_1","created_at":1,"object":"response","output":[]}}"#
        )));
    }
}
