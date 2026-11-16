use std::error::Error;
use std::fmt::Display;

use async_trait::async_trait;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_models::decisions::Json;
use tracing::{debug, error, warn};

use super::events::{buffer_holds_server_tool_use, rebuild_agentic_response};
use super::interceptor::{AgenticFollowUp, AgenticHookFuture, AgenticInterceptor};
use super::{
    MessagesAgenticError, MessagesAgenticStreamError, MessagesAgenticStreamInput,
    MessagesAgenticStreamResult, PING_INTERVAL_SECONDS, SERVER_FULFILLED_TOOL_LEAK_ERROR,
};
use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};
use crate::api::messages_sse_events::stream::MessagesProviderStream;
use crate::meta_muse_realtime::request::Sleep;

const UNKNOWN_CALL_ID: &str = "unknown";

// LiteLLM's AgenticAnthropicStreamingIterator: collect the provider bytes, then serve the hook's follow-up or the buffer
pub struct MessagesAgenticStream<S: MessagesProviderStream, I: AgenticInterceptor> {
    inner: S,
    interceptor: I,
    input: MessagesAgenticStreamInput,
    clock: Sleep,
    limits: ApiLimits,
    collected: Vec<Vec<u8>>,
    collected_bytes: usize,
    drain_started: bool,
    stream_exhausted: bool,
    hooks: HookState<I::FollowUp>,
    hook_processing_done: bool,
    follow_up: Option<I::FollowUp>,
    leak: Option<bool>,
    replay_index: usize,
    error_emitted: bool,
    closed: bool,
}

enum HookState<FollowUp> {
    NotStarted,
    Running(AgenticHookFuture<FollowUp>),
    Done,
}

impl<S, I> MessagesAgenticStream<S, I>
where
    S: MessagesProviderStream,
    I: AgenticInterceptor,
    S::Error: Error + 'static,
    <I::FollowUp as MessagesProviderStream>::Error: Error + 'static,
{
    pub fn new(
        inner: S,
        interceptor: I,
        input: MessagesAgenticStreamInput,
        clock: Sleep,
        limits: ApiLimits,
    ) -> Self {
        Self {
            inner,
            interceptor,
            input,
            clock,
            limits,
            collected: Vec::new(),
            collected_bytes: 0,
            drain_started: false,
            stream_exhausted: false,
            hooks: HookState::NotStarted,
            hook_processing_done: false,
            follow_up: None,
            leak: None,
            replay_index: 0,
            error_emitted: false,
            closed: false,
        }
    }

    // LiteLLM's aclose: settle the owned work, then close the streams it was handed
    pub async fn close(&mut self) -> Result<(), MessagesAgenticStreamError<S, I>> {
        self.closed = true;
        // dropping the stored hook future is LiteLLM's cancel of the hook task
        self.hooks = HookState::Done;
        if let Some(closed) = self.inner.aclose().await {
            closed.map_err(MessagesAgenticError::Provider)?;
        }
        if let Some(follow_up) = self.follow_up.as_mut()
            && let Some(closed) = follow_up.aclose().await
        {
            closed.map_err(MessagesAgenticError::FollowUp)?;
        }
        Ok(())
    }

    async fn read(&mut self) -> Result<Option<Vec<u8>>, MessagesAgenticStreamError<S, I>> {
        if self.closed {
            return Ok(None);
        }
        if self.input.hold_back.is_some() {
            return self.read_held_back().await;
        }
        self.read_live().await
    }

    // LiteLLM's __anext__ without hold_back: yield live, then process hooks at exhaustion
    async fn read_live(&mut self) -> Result<Option<Vec<u8>>, MessagesAgenticStreamError<S, I>> {
        if !self.stream_exhausted {
            let chunk = self
                .inner
                .next_chunk()
                .await
                .map_err(MessagesAgenticError::Provider)?;
            match chunk {
                Some(chunk) => {
                    self.collect(&chunk)?;
                    self.collected.push(chunk.clone());
                    return Ok(Some(chunk));
                }
                None => {
                    self.stream_exhausted = true;
                    self.run_hooks().await;
                }
            }
        }
        match self.follow_up.as_mut() {
            Some(follow_up) => follow_up
                .next_chunk()
                .await
                .map_err(MessagesAgenticError::FollowUp),
            None => Ok(None),
        }
    }

    fn collect(&mut self, chunk: &[u8]) -> Result<(), MessagesAgenticStreamError<S, I>> {
        let total = self.collected_bytes.saturating_add(chunk.len());
        self.limits.check(ApiBoundariesInput::Json(JsonBoundary {
            known_bytes: total,
        }))?;
        self.collected_bytes = total;
        Ok(())
    }

    fn ping_interval(&self) -> f64 {
        self.input
            .hold_back
            .as_ref()
            .map_or(PING_INTERVAL_SECONDS, |policy| policy.ping_interval_seconds)
    }

    fn leak_verdict(&mut self) -> Option<Vec<u8>> {
        if self.error_emitted {
            return None;
        }
        self.error_emitted = true;
        error!(
            "AgenticStreamingIterator: hooks did not replace a message containing a server-fulfilled tool_use [model={}]; emitting an SSE error instead of leaking the tool call to the client",
            self.input.model,
        );
        Some(SERVER_FULFILLED_TOOL_LEAK_ERROR.to_vec())
    }

    fn replay_next(&mut self) -> Option<Vec<u8>> {
        let chunk = self.collected.get(self.replay_index)?.clone();
        self.replay_index = self.replay_index.saturating_add(1);
        Some(chunk)
    }

    fn server_tool_holds(&mut self) -> Result<bool, MessagesAgenticStreamError<S, I>> {
        if let Some(holds) = self.leak {
            return Ok(holds);
        }
        let holds = match self.input.hold_back.as_ref() {
            Some(policy) => {
                let raw = self.collected.concat();
                buffer_holds_server_tool_use(
                    &raw,
                    &policy.server_fulfilled_tool_names,
                    &self.limits,
                )?
            }
            None => false,
        };
        self.leak = Some(holds);
        Ok(holds)
    }

    fn hook_future(&mut self) -> Option<AgenticHookFuture<I::FollowUp>> {
        if self.hook_processing_done {
            return None;
        }
        self.hook_processing_done = true;
        if self.collected.is_empty() {
            return None;
        }
        let raw = self.collected.concat();
        let response = match rebuild_agentic_response(&raw, &self.limits) {
            Ok(Some(response)) => response,
            Ok(None) => {
                debug!("AgenticStreamingIterator: Could not rebuild response from SSE bytes");
                return None;
            }
            Err(failure) => {
                self.log_hook_failure(&failure);
                return None;
            }
        };
        Some(self.interceptor.intercept(&response, &self.input))
    }

    fn finish_hooks(
        &mut self,
        outcome: Result<MessagesAgenticStreamResult<I::FollowUp>, AgenticHookError>,
    ) {
        match outcome {
            Ok(MessagesAgenticStreamResult::NoFollowUp) => {}
            Ok(MessagesAgenticStreamResult::Stream(follow_up)) => {
                self.follow_up = Some(follow_up);
            }
            Ok(MessagesAgenticStreamResult::Message(message)) => {
                match I::FollowUp::from_response(&message) {
                    Ok(follow_up) => self.follow_up = Some(follow_up),
                    Err(failure) => self.log_hook_failure(&failure),
                }
            }
            Ok(MessagesAgenticStreamResult::Unexpected { type_name }) => {
                warn!("AgenticStreamingIterator: Unexpected result type from hooks: {type_name}");
            }
            Err(failure) => self.log_hook_failure(&failure),
        }
        self.hooks = HookState::Done;
    }

    fn log_hook_failure(&self, failure: &dyn Display) {
        let call_id = self
            .input
            .logging
            .call_id
            .as_deref()
            .unwrap_or(UNKNOWN_CALL_ID);
        error!(
            "AgenticStreamingIterator: Error in agentic hook processing [call_id={} model={}]: {}",
            call_id, self.input.model, failure,
        );
    }
}

#[async_trait]
impl<S, I> MessagesProviderStream for MessagesAgenticStream<S, I>
where
    S: MessagesProviderStream + Send,
    I: AgenticInterceptor,
    S::Error: Error + 'static,
    <I::FollowUp as MessagesProviderStream>::Error: Error + 'static,
{
    type Error = MessagesAgenticStreamError<S, I>;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Self::Error> {
        self.read().await
    }

    async fn aclose(&mut self) -> Option<Result<(), Self::Error>> {
        Some(self.close().await)
    }

    // LiteLLM's has_buffered_provider_output: provider bytes received but withheld behind pings
    fn buffered_provider_output(&self) -> Option<Json> {
        Some(Json::Bool(
            self.input.hold_back.is_some() && !self.collected.is_empty(),
        ))
    }
}

mod held_back;
