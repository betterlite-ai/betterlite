use std::collections::VecDeque;
use std::pin::Pin;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::BaseTranslation;
use betterlite_guardrails::unified_dispatch::stream_items::{
    chunk_carries_text, chunk_has_finish_reason, chunk_has_tool_calls, held_choices,
    is_trailing_metadata_chunk, record_finish_reasons, tool_call_passthrough_chunk,
};
use futures_util::StreamExt as _;
use futures_util::stream::{Stream, unfold};

use super::error::{RoundFailure, StreamLoopError};
use super::round::{self, TransformState};
use super::{StreamBuffers, StreamLoopParams, StreamOutput, inspect, tail};

#[derive(Default)]
struct Seen {
    tool_calls: bool,
    text_content: bool,
}

// Everything one run remembers between source items; the Python locals of the loop
pub(crate) struct LoopState<'run, T, G, S> {
    params: StreamLoopParams<'run, T, G>,
    source: Pin<Box<S>>,
    transform: TransformState,
    responses_so_far: Vec<GuardrailValue>,
    responses_yielded: Vec<GuardrailValue>,
    outputs: VecDeque<StreamOutput>,
    last_chunk: Option<GuardrailValue>,
    chunk_counter: usize,
    seen: Seen,
    failure: Option<StreamLoopError>,
    finished: bool,
}

// LiteLLM's _run_incremental_transform_stream: withhold the raw chunks, send the transformed diffs
#[must_use]
pub fn run_incremental_transform_stream<'run, T, G, S>(
    params: StreamLoopParams<'run, T, G>,
    source: S,
) -> impl Stream<Item = Result<StreamOutput, StreamLoopError>> + Send + 'run
where
    T: BaseTranslation<Guardrail = G, LogTarget = GuardrailLogTarget, Failure = GuardrailFailure>
        + Sync
        + 'run,
    G: GuardrailContract + 'run,
    S: Stream<Item = GuardrailValue> + Send + 'run,
{
    let start = LoopState {
        params,
        source: Box::pin(source),
        transform: TransformState::default(),
        responses_so_far: Vec::new(),
        responses_yielded: Vec::new(),
        outputs: VecDeque::new(),
        last_chunk: None,
        chunk_counter: 0,
        seen: Seen::default(),
        failure: None,
        finished: false,
    };
    unfold(start, |mut state| async move {
        loop {
            if let Some(item) = state.outputs.pop_front() {
                return Some((Ok(item), state));
            }
            if let Some(error) = state.failure.take() {
                return Some((Err(error), state));
            }
            if state.finished {
                return None;
            }
            state.advance().await;
        }
    })
}

impl<'run, T, G, S> LoopState<'run, T, G, S>
where
    T: BaseTranslation<Guardrail = G, LogTarget = GuardrailLogTarget, Failure = GuardrailFailure>
        + Sync,
    G: GuardrailContract,
    S: Stream<Item = GuardrailValue> + Send,
{
    async fn advance(&mut self) {
        let pulled = self.source.next().await;
        match pulled {
            Some(item) => self.consume_item(item).await,
            None => self.finish_stream().await,
        }
    }

    async fn consume_item(&mut self, item: GuardrailValue) {
        if chunk_has_tool_calls(&item) {
            self.consume_tool_call_chunk(item).await;
        } else if is_trailing_metadata_chunk(&item) {
            self.responses_so_far.push(item);
        } else {
            self.consume_text_chunk(item).await;
        }
    }

    async fn consume_tool_call_chunk(&mut self, item: GuardrailValue) {
        self.seen.tool_calls = true;
        self.responses_so_far.push(item.clone());
        self.last_chunk = Some(item.clone());
        if self.seen.text_content {
            let flushed = self.run_round(&item, false).await;
            if let Err(failure) = flushed {
                self.absorb(failure);
                return;
            }
        }
        let held = held_choices(&self.transform.held_chars);
        let passthrough =
            tool_call_passthrough_chunk(&item, Some(&mut self.transform.finish_reasons), &held);
        self.responses_yielded.push(passthrough.clone());
        self.outputs.push_back(StreamOutput::Item(passthrough));
    }

    async fn consume_text_chunk(&mut self, item: GuardrailValue) {
        self.chunk_counter = self.chunk_counter.saturating_add(1);
        self.responses_so_far.push(item.clone());
        self.last_chunk = Some(item.clone());
        record_finish_reasons(&item, &mut self.transform.finish_reasons);
        if chunk_carries_text(&item) {
            self.seen.text_content = true;
        }
        if self.round_is_sampled(&item) {
            let sampled = self.run_round(&item, false).await;
            if let Err(failure) = sampled {
                self.absorb(failure);
            }
        }
    }

    // a terminal chunk is processed once by the end-of-stream flush, never by a sampled round
    fn round_is_sampled(&self, item: &GuardrailValue) -> bool {
        !self.params.end_of_stream_only
            && !chunk_has_finish_reason(item)
            && self.chunk_counter.checked_rem(self.params.sampling_rate) == Some(0)
    }

    async fn run_round(
        &mut self,
        reference: &GuardrailValue,
        is_final: bool,
    ) -> Result<(), RoundFailure> {
        round::emit_transform_round(
            &mut self.params,
            &mut self.transform,
            StreamBuffers {
                responses_so_far: &mut self.responses_so_far,
                responses_yielded: &mut self.responses_yielded,
                outputs: &mut self.outputs,
            },
            reference,
            is_final,
        )
        .await
    }

    // the terminal chunks were already queued; a loop failure still leaves as a stream error
    fn absorb(&mut self, failure: RoundFailure) {
        if let RoundFailure::Loop(error) = failure {
            self.failure = Some(error);
        }
        self.finished = true;
    }

    async fn finish_stream(&mut self) {
        if self.seen.tool_calls {
            let mut assembled = self.responses_so_far.clone();
            let inspected = inspect::inspect_full_response_for_block(
                &mut self.params,
                &mut assembled,
                &self.responses_yielded,
                &mut self.outputs,
            )
            .await;
            if let Err(failure) = inspected {
                self.absorb(failure);
                return;
            }
        }
        let last_chunk = self.last_chunk.clone();
        let tailed = tail::emit_stream_tail(
            &mut self.params,
            &mut self.transform,
            StreamBuffers {
                responses_so_far: &mut self.responses_so_far,
                responses_yielded: &mut self.responses_yielded,
                outputs: &mut self.outputs,
            },
            last_chunk.as_ref(),
        )
        .await;
        self.absorb(tailed);
    }
}

mod tests;
