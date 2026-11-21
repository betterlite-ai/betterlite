use std::error::Error;
use std::mem;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::BaseTranslation;
use betterlite_guardrails::unified_dispatch::emit::{
    HttpErrorDetail, StreamErrorEmission, StreamHttpError, emit_streaming_http_error,
    handle_streaming_block,
};
use betterlite_guardrails::unified_dispatch::flags::is_redundant_scan;

use super::block_only::{Scanner, Window};
use super::error::StreamHookError;
use crate::guardrails::unified_stream::error::StreamLoopError;
use crate::guardrails::unified_stream::round::call_streaming;
use crate::guardrails::unified_stream::{StreamLoopParams, StreamOutput};

// The withheld chunks are copies the scan never rewrites, so the originals go out whatever release_on_scan says
pub(crate) async fn finish_stream<T, G, E>(window: &mut Window<E>, scanner: &mut Scanner<'_, T, G>)
where
    T: BaseTranslation<Guardrail = G, LogTarget = GuardrailLogTarget, Failure = GuardrailFailure>
        + Sync,
    G: GuardrailContract,
    E: Error + Send + Sync + 'static,
{
    let name = scanner.guardrail_name;
    tracing::debug!(
        "Processing final streaming response with all {} chunks for guardrail {name}",
        window.responses_so_far.len()
    );
    let buffered = mem::take(&mut window.withheld_items);
    let end_key = scanner
        .params
        .translation
        .get_streaming_scan_key(&window.responses_so_far);
    if is_redundant_scan(end_key.as_ref(), window.last_scan_key.as_ref()) {
        tracing::debug!(
            "Skipping end-of-stream scan for guardrail {name}: the last sampled round already scanned it all"
        );
    } else {
        let scanned = call_streaming(&mut scanner.params, &mut window.responses_so_far, None).await;
        if let Err(failure) = scanned {
            window.terminate(&scanner.params, failure);
            return;
        }
    }
    for item in buffered {
        window.outputs.push_back(Ok(StreamOutput::Item(item)));
    }
    for item in mem::take(&mut window.pending_items) {
        window.send(item);
    }
}

impl<E> Window<E>
where
    E: Error + Send + Sync + 'static,
{
    // LiteLLM's ModifyResponseException and HTTPException handlers; the block measures against what the client got
    pub(crate) fn terminate<T, G>(
        &mut self,
        params: &StreamLoopParams<'_, T, G>,
        failure: GuardrailFailure,
    ) where
        T: BaseTranslation<Failure = GuardrailFailure>,
    {
        let stream_started = !self.responses_yielded.is_empty();
        let http_error = match failure {
            GuardrailFailure::ModifyResponse(details) => {
                self.push_block(params.translation, details, stream_started);
                return;
            }
            GuardrailFailure::Http { status, detail } => StreamHttpError {
                status_code: status,
                detail: HttpErrorDetail::Text(detail),
            },
            GuardrailFailure::HttpObject { status, detail } => StreamHttpError {
                status_code: status,
                detail: HttpErrorDetail::Object(detail.into_iter().collect()),
            },
            other @ (GuardrailFailure::GuardrailRaised { .. }
            | GuardrailFailure::BlockedPii { .. }
            | GuardrailFailure::SensitiveDataRoute(_)
            | GuardrailFailure::Other(_)) => {
                let failed = StreamLoopError::Guardrail { source: other };
                self.outputs.push_back(Err(StreamHookError::Loop(failed)));
                return;
            }
        };
        self.emit_http(params, http_error, stream_started);
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
            details.original_response = Some(GuardrailValue::Array(self.responses_so_far.clone()));
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
        error: StreamHttpError,
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
        match emit_streaming_http_error(error, &emission) {
            Ok(items) => {
                for item in items {
                    self.outputs.push_back(Ok(StreamOutput::Item(item)));
                }
            }
            Err(source) => {
                let failed = StreamLoopError::Http { source };
                self.outputs.push_back(Err(StreamHookError::Loop(failed)));
            }
        }
    }
}
