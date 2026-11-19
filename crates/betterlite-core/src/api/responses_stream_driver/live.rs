use betterlite_error::error::ErrorClass;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use indexmap::IndexMap;
use num_traits::ToPrimitive as _;

use super::errors::{DriverError, TransportFailure};
use super::fallback::{
    context, fallback_error, note_yielded_event, source_error, stream_ended_early_message,
    timeout_class,
};
use super::source::PayloadSource;
use crate::api::api_boundaries::ApiLimits;
use crate::api::response_synthetic_events::estimate::UsageTokenization;
use crate::api::response_synthetic_events::stamping::ResponsesCostCalculator;
use crate::api::responses_stream_events::{
    BaseStreamStateInput, ResponsesEventProcessor, ResponsesEventTransform,
    ResponsesStreamBaseState, base_stream_state,
};
use crate::api::responses_stream_hooks::ResponsesStreamHooks;
use crate::api::responses_stream_hooks::deployment::{ChunkHookContext, chunk_hook};
use crate::api::responses_stream_hooks::failed::maybe_raise_for_error_event;
use crate::api::responses_stream_hooks::failure::{await_pending_logging, handle_failure};
use crate::api::responses_stream_hooks::ports::Dispatch;
use crate::api::responses_stream_hooks::state::ResponsesStreamHookState;

pub type Event = ResponsesAPIStreamingResponse;

pub struct StreamEngine<'stream, Transform, Tokenization, Calculator>
where
    Transform: ResponsesEventTransform + Sync,
    Tokenization: UsageTokenization + Sync,
    Calculator: ResponsesCostCalculator + Sync,
{
    pub transform: Option<&'stream Transform>,
    pub tokenization: Option<&'stream Tokenization>,
    pub calculator: Option<&'stream Calculator>,
    pub hooks: &'stream ResponsesStreamHooks<'stream>,
    pub logging: &'stream StreamLoggingObject,
    pub details: IndexMap<String, Json>,
    pub limits: &'stream ApiLimits,
}
pub struct LiveStream<'stream, Transform, Tokenization, Calculator>
where
    Transform: ResponsesEventTransform + Sync,
    Tokenization: UsageTokenization + Sync,
    Calculator: ResponsesCostCalculator + Sync,
{
    engine: StreamEngine<'stream, Transform, Tokenization, Calculator>,
    state: ResponsesStreamBaseState,
    pub callback: ResponsesStreamHookState,
    source: Box<dyn PayloadSource>,
}
impl<'stream, Transform, Tokenization, Calculator>
    LiveStream<'stream, Transform, Tokenization, Calculator>
where
    Transform: ResponsesEventTransform + Sync,
    Tokenization: UsageTokenization + Sync,
    Calculator: ResponsesCostCalculator + Sync,
{
    #[must_use]
    pub fn new(
        engine: StreamEngine<'stream, Transform, Tokenization, Calculator>,
        input: BaseStreamStateInput<'_>,
        source: Box<dyn PayloadSource>,
    ) -> Self {
        Self {
            state: base_stream_state(&input),
            callback: ResponsesStreamHookState::default(),
            engine,
            source,
        }
    }
    // The async iterator awaits the hook directly and waits for pending logging on every exit.
    pub async fn next_event(&mut self) -> Result<Option<Event>, DriverError> {
        let outcome = self.drive(Dispatch::Spawned).await;
        if matches!(&outcome, Ok(None) | Err(_)) {
            await_pending_logging(&mut self.callback).await;
        }
        outcome
    }
    // both iterators share this pipeline and only the dispatch differs
    pub async fn drive(&mut self, dispatch: Dispatch) -> Result<Option<Event>, DriverError> {
        match self.pull(dispatch).await {
            Ok(event) => Ok(event),
            Err(DriverError::Transport(failure)) => self.transport_failed(failure).await,
            Err(DriverError::Class { class }) => {
                self.state.finished = true;
                handle_failure(
                    self.engine.hooks,
                    &mut self.state,
                    &mut self.callback,
                    &class,
                )
                .await;
                Err(DriverError::Class { class })
            }
        }
    }
    async fn pull(&mut self, dispatch: Dispatch) -> Result<Option<Event>, DriverError> {
        self.check_duration()?;
        loop {
            let Some(item) = self.source.next_payload().await else {
                self.state.finished = true;
                self.raise_if_ended().await?;
                return Ok(None);
            };
            let payload = item.map_err(source_error)?;
            self.check_duration()?;
            // httpx decodes the body with replacement characters, so an unreadable frame skips like malformed JSON
            let text = String::from_utf8_lossy(&payload);
            let outcome = {
                let mut processor = ResponsesEventProcessor {
                    transform: self.engine.transform,
                    tokenization: self.engine.tokenization,
                    calculator: self.engine.calculator,
                    hooks: self.engine.hooks,
                    details: &mut self.engine.details,
                    dispatch,
                    limits: self.engine.limits,
                };
                processor
                    .process(
                        text.as_ref(),
                        &mut self.state,
                        &mut self.callback,
                        self.engine.logging,
                    )
                    .await
            };
            let result = outcome.map_err(|error| DriverError::Class {
                class: error
                    .error_class(&self.state.model, self.state.custom_llm_provider.as_deref()),
            })?;
            if self.state.finished {
                self.raise_if_ended().await?;
                return Ok(None);
            }
            if let Some(event) = result {
                if let Err(failure) =
                    maybe_raise_for_error_event(self.engine.hooks, &self.state, &event)
                {
                    return Err(DriverError::Class {
                        class: failure.exception,
                    });
                }
                let context = ChunkHookContext {
                    explicit_call_type: self.state.call_type.as_deref(),
                    logging_call_type: None,
                    request_data: Some(&self.state.request_data),
                    model_call_details: &self.engine.details,
                };
                let event = chunk_hook(self.engine.hooks, &mut self.callback, context, event).await;
                note_yielded_event(&mut self.state, &event);
                return Ok(Some(event));
            }
        }
    }
    fn check_duration(&self) -> Result<(), DriverError> {
        self.state
            .check_max_streaming_duration(self.clock_seconds())
            .map_err(|timeout| DriverError::Class {
                class: timeout_class(&timeout),
            })
    }
    fn clock_seconds(&self) -> f64 {
        self.engine
            .hooks
            .clock
            .now()
            .timestamp_micros()
            .to_f64()
            .map(|micros| micros.mul_add(1e-6, 0.0_f64))
            .unwrap_or(f64::INFINITY)
    }
    async fn raise_if_ended(&mut self) -> Result<(), DriverError> {
        if self.state.completed_response.is_some() {
            return Ok(());
        }
        let message = stream_ended_early_message(&self.state);
        let class_message = format!("litellm.APIConnectionError: {message}");
        let class =
            ErrorClass::ApiConnection(Box::new(context(class_message.clone(), &self.state)));
        handle_failure(
            self.engine.hooks,
            &mut self.state,
            &mut self.callback,
            &class,
        )
        .await;
        if self.state.output_started {
            return Err(DriverError::Class { class });
        }
        Err(DriverError::Class {
            class: fallback_error(&self.state, &class_message, 500),
        })
    }
    async fn transport_failed(
        &mut self,
        failure: TransportFailure,
    ) -> Result<Option<Event>, DriverError> {
        self.state.finished = true;
        if failure.is_transport() && self.state.completed_response.is_some() {
            return Ok(None);
        }
        let class =
            ErrorClass::ApiConnection(Box::new(context(failure.message.clone(), &self.state)));
        handle_failure(
            self.engine.hooks,
            &mut self.state,
            &mut self.callback,
            &class,
        )
        .await;
        if !failure.is_transport() || self.state.output_started {
            return Err(DriverError::Transport(failure));
        }
        Err(DriverError::Class {
            class: fallback_error(&self.state, &failure.message, 503),
        })
    }
}
