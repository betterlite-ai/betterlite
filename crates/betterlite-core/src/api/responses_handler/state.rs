use std::collections::VecDeque;

use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_context::non_default_params::Params;
use betterlite_interactions::types::response::InteractionsStreamChunk;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::responses_event_records::event_types::OutputTextDeltaEvent;
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse as UpstreamEvent,
};
use thiserror::Error;

use super::events::{
    completion_event, content_start_event, content_stop_event, interaction_start_event,
    text_delta_event,
};
use crate::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, EventBoundary, JsonBoundary,
};
use crate::api::messages_response_events::StreamFlag;

// The state's failures: the collected text or the completion event carrying it reached a boundary, or that event could not be encoded.
#[derive(Debug, Error)]
pub enum InteractionsStreamStateError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the synthesized interaction event could not be encoded")]
    Encode(#[source] serde_json::Error),
}

// The constructor arguments the handlers pass on, plus the id source the fallback interaction id comes from and the boundary limits the text stays under.
pub struct IteratorSetup<'call> {
    pub model: &'call str,
    pub request_input: Option<&'call JsonValue>,
    pub optional_params: &'call Params,
    pub custom_llm_provider: Option<&'call str>,
    pub litellm_metadata: &'call Params,
    pub ids: &'call dyn IdSource,
    pub limits: ApiLimits,
}

impl IteratorSetup<'_> {
    // A copy with the provider the built request carries, which is where LiteLLM's async branch reads it.
    #[must_use]
    pub fn with_provider<'call>(&'call self, provider: Option<&'call str>) -> IteratorSetup<'call> {
        IteratorSetup {
            model: self.model,
            request_input: self.request_input,
            optional_params: self.optional_params,
            custom_llm_provider: provider,
            litellm_metadata: self.litellm_metadata,
            ids: self.ids,
            limits: self.limits,
        }
    }
}

// LiteLLMResponsesInteractionsStreamingIterator's record: what __init__ stored, the collected text, the send flags and the queue iteration drains.
pub struct InteractionsStreamState {
    pub model: String,
    pub request_input: Option<JsonValue>,
    pub optional_params: Params,
    pub custom_llm_provider: Option<String>,
    pub litellm_metadata: Params,
    pub collected_text: String,
    pub sent_interaction_start: StreamFlag,
    pub sent_content_start: StreamFlag,
    pub sent_completion_event: StreamFlag,
    pub finished: StreamFlag,
    pub pending_events: VecDeque<InteractionsStreamChunk>,
    pub interaction_id: Option<String>,
    pub fallback_id: String,
    pub limits: ApiLimits,
}

impl InteractionsStreamState {
    // __init__ draws the fallback once from the injected source, where LiteLLM keeps id(self)'s address, so every event stays correlatable.
    #[must_use]
    pub fn new(setup: &IteratorSetup<'_>) -> Self {
        Self {
            model: setup.model.to_owned(),
            request_input: setup.request_input.cloned(),
            optional_params: setup.optional_params.clone(),
            custom_llm_provider: setup.custom_llm_provider.map(str::to_owned),
            litellm_metadata: setup.litellm_metadata.clone(),
            collected_text: String::new(),
            sent_interaction_start: StreamFlag(false),
            sent_content_start: StreamFlag(false),
            sent_completion_event: StreamFlag(false),
            finished: StreamFlag(false),
            pending_events: VecDeque::new(),
            interaction_id: None,
            fallback_id: format!("interaction_{}", setup.ids.next_id().simple()),
            limits: setup.limits,
        }
    }

    // _events_for_chunk: only the four chunks LiteLLM's isinstance checks name produce events, and any other chunk produces none.
    pub fn events_for_chunk(
        &mut self,
        chunk: &UpstreamEvent,
    ) -> Result<Vec<InteractionsStreamChunk>, InteractionsStreamStateError> {
        let UpstreamEvent::Known(known) = chunk else {
            return Ok(Vec::new());
        };
        if let KnownResponseEvent::OutputTextDelta(delta) = known.as_ref() {
            return self.delta_events(delta);
        }
        if let KnownResponseEvent::ResponseCreated(created) = known.as_ref() {
            return Ok(self.start_events(&created.response.id));
        }
        if let KnownResponseEvent::ResponseInProgress(in_progress) = known.as_ref() {
            return Ok(self.start_events(&in_progress.response.id));
        }
        if let KnownResponseEvent::ResponseCompleted(completed) = known.as_ref() {
            return Ok(self.completed_events(&completed.response.id));
        }
        Ok(Vec::new())
    }

    // _build_terminal_events_on_eof: a stream that ends without response.completed still ends with step.stop and interaction.completed.
    #[must_use]
    pub fn terminal_events_on_eof(&mut self) -> Vec<InteractionsStreamChunk> {
        if self.sent_completion_event.0 {
            return Vec::new();
        }
        let interaction_id = self
            .interaction_id
            .clone()
            .unwrap_or_else(|| self.fallback_id.clone());
        let mut terminal = Vec::new();
        if self.sent_content_start.0 {
            terminal.push(content_stop_event());
        }
        if self.sent_interaction_start.0 || !self.collected_text.is_empty() {
            terminal.push(completion_event(
                &interaction_id,
                &self.model,
                &self.collected_text,
            ));
            self.sent_completion_event = StreamFlag(true);
        }
        terminal
    }

    // The text delta arm: store the item id when it is the first id seen, refuse text past the boundaries, open whatever start events are missing, then append the delta.
    fn delta_events(
        &mut self,
        delta: &OutputTextDeltaEvent,
    ) -> Result<Vec<InteractionsStreamChunk>, InteractionsStreamStateError> {
        self.collected_text.push_str(&delta.delta);
        let interaction_id =
            provided_id(&delta.item_id).unwrap_or_else(|| self.fallback_id.clone());
        if self.interaction_id.is_none() {
            self.interaction_id = Some(interaction_id.clone());
        }
        self.limits.check(ApiBoundariesInput::Json(JsonBoundary {
            known_bytes: self.collected_text.len(),
        }))?;
        // the terminal event uses the stored id, so the bounded event is the one the stream ends with
        let terminal_id = self.interaction_id.as_deref().unwrap_or(&self.fallback_id);
        let completion = serde_json::to_vec(&completion_event(
            terminal_id,
            &self.model,
            &self.collected_text,
        ))
        .map_err(InteractionsStreamStateError::Encode)?;
        self.limits.check(ApiBoundariesInput::Event(EventBoundary {
            known_bytes: completion.len(),
        }))?;
        let mut events = Vec::new();
        if !self.sent_interaction_start.0 {
            self.sent_interaction_start = StreamFlag(true);
            events.push(interaction_start_event(&interaction_id, &self.model));
        }
        if !self.sent_content_start.0 {
            self.sent_content_start = StreamFlag(true);
            events.push(content_start_event());
        }
        events.push(text_delta_event(&delta.delta));
        Ok(events)
    }

    // The response.created and response.in_progress arm: one interaction start, and only the first time either arrives.
    fn start_events(&mut self, response_id: &str) -> Vec<InteractionsStreamChunk> {
        if self.sent_interaction_start.0 {
            return Vec::new();
        }
        self.sent_interaction_start = StreamFlag(true);
        let interaction_id = provided_id(response_id).unwrap_or_else(|| self.fallback_id.clone());
        if self.interaction_id.is_none() {
            self.interaction_id = Some(interaction_id.clone());
        }
        vec![interaction_start_event(&interaction_id, &self.model)]
    }

    // The response.completed arm: the stored id wins over response.id, so the completion matches the start events already sent.
    fn completed_events(&mut self, response_id: &str) -> Vec<InteractionsStreamChunk> {
        self.finished = StreamFlag(true);
        let interaction_id = self
            .interaction_id
            .clone()
            .or_else(|| provided_id(response_id))
            .unwrap_or_else(|| self.fallback_id.clone());
        let mut terminal = Vec::new();
        if self.sent_content_start.0 {
            terminal.push(content_stop_event());
        }
        terminal.push(completion_event(
            &interaction_id,
            &self.model,
            &self.collected_text,
        ));
        self.sent_completion_event = StreamFlag(true);
        terminal
    }
}

// LiteLLM's `getattr(responses_chunk, "item_id", None) or ...`: an empty id is no id as far as correlation goes.
fn provided_id(id: &str) -> Option<String> {
    if id.is_empty() {
        return None;
    }
    Some(id.to_owned())
}
