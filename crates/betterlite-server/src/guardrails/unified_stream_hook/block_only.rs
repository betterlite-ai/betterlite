use std::collections::VecDeque;
use std::error::Error;
use std::mem;
use std::ops::ControlFlow;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::stream_item_value;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, StreamingScanKey};
use betterlite_guardrails::unified_dispatch::emit::{
    HttpErrorDetail, StreamErrorEmission, StreamHttpError, emit_streaming_http_error,
    handle_streaming_block,
};
use betterlite_guardrails::unified_dispatch::flags::is_redundant_scan;

use super::error::StreamHookError;
use super::stream_end::StreamEndCheck;
use crate::guardrails::unified_stream::error::StreamLoopError;
use crate::guardrails::unified_stream::round::call_streaming;
use crate::guardrails::unified_stream::{StreamLoopParams, StreamOutput};

pub(crate) type HookOutput<E> = Result<StreamOutput, StreamHookError<E>>;

// The translation the first chunk resolved, with what every round reads
pub(crate) struct Scanner<'run, T, G> {
    pub(crate) params: StreamLoopParams<'run, T, G>,
    pub(crate) buffering: bool,
    pub(crate) guardrail_name: &'run str,
}

// LiteLLM's loop locals; its chunks_yielded is always !responses_yielded.is_empty(), so only the list is kept
pub(crate) struct Window<E> {
    pub(crate) chunk_counter: usize,
    pub(crate) responses_so_far: Vec<StreamItem>,
    pub(crate) responses_yielded: Vec<StreamItem>,
    pub(crate) withheld_items: Vec<StreamItem>,
    pub(crate) pending_items: Vec<StreamItem>,
    pub(crate) last_scan_key: Option<StreamingScanKey>,
    pub(crate) tool_calls_in_flight: bool,
    pub(crate) outputs: VecDeque<HookOutput<E>>,
}

impl<E> Window<E>
where
    E: Error + Send + Sync + 'static,
{
    pub(crate) async fn scan_item<T, G>(
        &mut self,
        scanner: &mut Scanner<'_, T, G>,
        item: StreamItem,
    ) -> ControlFlow<()>
    where
        T: BaseTranslation<
                Guardrail = G,
                LogTarget = GuardrailLogTarget,
                Failure = GuardrailFailure,
            > + StreamEndCheck
            + Sync,
        G: GuardrailContract,
    {
        self.chunk_counter = self.chunk_counter.saturating_add(1);
        self.responses_so_far.push(item.clone());
        if scanner.params.end_of_stream_only {
            self.hold_for_end(scanner, item);
            return ControlFlow::Continue(());
        }
        if scanner.buffering {
            self.withheld_items.push(item.clone());
        }
        if self.chunk_counter.checked_rem(scanner.params.sampling_rate) == Some(0) {
            return self.sampled_round(scanner, item).await;
        }
        if !scanner.buffering {
            self.send(item);
        }
        ControlFlow::Continue(())
    }

    // a chunk after the stream ended waits for the final scan, so a block can replace the finish
    fn hold_for_end<T, G>(&mut self, scanner: &Scanner<'_, T, G>, item: StreamItem)
    where
        T: StreamEndCheck,
    {
        if scanner.buffering {
            self.withheld_items.push(item);
        } else if !self.pending_items.is_empty()
            || scanner
                .params
                .translation
                .streaming_has_ended(&self.responses_so_far)
        {
            self.pending_items.push(item);
        } else {
            self.send(item);
        }
    }

    async fn sampled_round<T, G>(
        &mut self,
        scanner: &mut Scanner<'_, T, G>,
        item: StreamItem,
    ) -> ControlFlow<()>
    where
        T: BaseTranslation<
                Guardrail = G,
                LogTarget = GuardrailLogTarget,
                Failure = GuardrailFailure,
            > + Sync,
        G: GuardrailContract,
    {
        let scan_key = scanner
            .params
            .translation
            .get_streaming_scan_key(&self.responses_so_far);
        if let Some(key) = &scan_key {
            self.tool_calls_in_flight = key.tool_calls_in_flight;
        }
        let hold_window = scanner.buffering && (scan_key.is_none() || self.tool_calls_in_flight);
        if is_redundant_scan(scan_key.as_ref(), self.last_scan_key.as_ref()) {
            tracing::debug!(
                "Skipping streaming chunk {} for guardrail {}: nothing new to scan since the last round",
                self.chunk_counter,
                scanner.guardrail_name
            );
        } else {
            self.scan_round(scanner, scan_key, hold_window).await?;
        }
        if hold_window {
            return ControlFlow::Continue(());
        }
        if scanner.buffering {
            for withheld in mem::take(&mut self.withheld_items) {
                self.send(withheld);
            }
        } else {
            self.send(item);
        }
        ControlFlow::Continue(())
    }

    async fn scan_round<T, G>(
        &mut self,
        scanner: &mut Scanner<'_, T, G>,
        scan_key: Option<StreamingScanKey>,
        hold_window: bool,
    ) -> ControlFlow<()>
    where
        T: BaseTranslation<
                Guardrail = G,
                LogTarget = GuardrailLogTarget,
                Failure = GuardrailFailure,
            > + Sync,
        G: GuardrailContract,
    {
        let name = scanner.guardrail_name;
        tracing::debug!(
            "Processing streaming chunk {} (sampling_rate={}) with guardrail {name}",
            self.chunk_counter,
            scanner.params.sampling_rate
        );
        let outcome = call_streaming(&mut scanner.params, &mut self.responses_so_far, None).await;
        if let Err(failure) = outcome {
            self.terminate(&scanner.params, failure);
            return ControlFlow::Break(());
        }
        if scan_key.is_some() {
            self.last_scan_key = scan_key;
        }
        if hold_window {
            tracing::debug!(
                "Holding {} buffered chunks for guardrail {name}: this round could not scan the whole window",
                self.withheld_items.len()
            );
        }
        ControlFlow::Continue(())
    }

    pub(crate) fn send(&mut self, item: StreamItem) {
        self.responses_yielded.push(item.clone());
        self.outputs.push_back(Ok(StreamOutput::Item(item)));
    }

    // LiteLLM's ModifyResponseException and HTTPException handlers; the block measures against what the client got
    pub(crate) fn terminate<T, G>(
        &mut self,
        params: &StreamLoopParams<'_, T, G>,
        failure: GuardrailFailure,
    ) where
        T: BaseTranslation<Failure = GuardrailFailure>,
    {
        let stream_started = !self.responses_yielded.is_empty();
        match failure {
            GuardrailFailure::ModifyResponse(details) => {
                self.push_block(params.translation, details, stream_started);
            }
            GuardrailFailure::Http { status, detail } => {
                self.emit_http(
                    params,
                    status,
                    HttpErrorDetail::Text(detail),
                    stream_started,
                );
            }
            GuardrailFailure::HttpObject { status, detail } => {
                let fields = HttpErrorDetail::Object(detail.into_iter().collect());
                self.emit_http(params, status, fields, stream_started);
            }
            other @ (GuardrailFailure::GuardrailRaised { .. }
            | GuardrailFailure::BlockedPii { .. }
            | GuardrailFailure::SensitiveDataRoute(_)
            | GuardrailFailure::Other(_)) => {
                let failed = StreamLoopError::Guardrail { source: other };
                self.outputs.push_back(Err(StreamHookError::Loop(failed)));
            }
        }
    }

    fn push_block<T>(
        &mut self,
        translation: &T,
        mut details: Box<ModifyResponseFailure>,
        stream_started: bool,
    ) where
        T: BaseTranslation<Failure = GuardrailFailure>,
    {
        if details.original_response.is_none() {
            details.original_response = Some(GuardrailValue::Array(
                self.responses_so_far
                    .iter()
                    .map(stream_item_value)
                    .collect(),
            ));
        }
        let blocked = GuardrailFailure::ModifyResponse(details);
        let chunks = handle_streaming_block(
            translation,
            blocked,
            stream_started,
            &self.responses_yielded,
        )
        .map(StreamOutput::BlockChunks)
        .map_err(|source| StreamHookError::Loop(StreamLoopError::Block { source }));
        self.outputs.push_back(chunks);
    }

    fn emit_http<T, G>(
        &mut self,
        params: &StreamLoopParams<'_, T, G>,
        status_code: u16,
        detail: HttpErrorDetail,
        stream_started: bool,
    ) where
        T: BaseTranslation<Failure = GuardrailFailure>,
    {
        let emission = StreamErrorEmission {
            call_type: Some(params.call_type),
            responses_so_far: &self.responses_so_far,
            request_data: &*params.request_data,
            translation: Some(params.translation),
            stream_started,
            responses_yielded: Some(&self.responses_yielded),
        };
        let error = StreamHttpError {
            status_code,
            detail,
        };
        let emitted = emit_streaming_http_error(error, &emission)
            .map_err(|source| StreamHookError::Loop(StreamLoopError::Http { source }));
        match emitted {
            Ok(items) => {
                let sent = items.into_iter().map(|item| Ok(StreamOutput::Item(item)));
                self.outputs.extend(sent);
            }
            Err(failed) => self.outputs.push_back(Err(failed)),
        }
    }
}
