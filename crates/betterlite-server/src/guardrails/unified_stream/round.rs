use std::collections::{BTreeMap, VecDeque};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, StreamTransformSink, TranslationCall,
};
use betterlite_guardrails::unified_dispatch::emit::{
    HttpErrorDetail, StreamErrorEmission, StreamHttpError, emit_streaming_http_error,
    handle_streaming_block,
};
use betterlite_guardrails::unified_dispatch::transform_chunk::{
    TransformChunkInput, build_transform_chunk,
};
use indexmap::IndexMap;

use super::error::{RoundFailure, StreamLoopError, StreamTerminated};
use super::{StreamBuffers, StreamLoopParams, StreamOutput};

// The maps one run of the loop threads through every round
#[derive(Debug, Default)]
pub(crate) struct TransformState {
    pub(crate) emitted_text: IndexMap<i64, String>,
    pub(crate) finish_reasons: IndexMap<i64, Option<String>>,
    pub(crate) held_chars: IndexMap<i64, i64>,
}

// Both calls of a round sit in one error scope, as in LiteLLM's single try block
enum RoundCallError {
    Guardrail(GuardrailFailure),
    Http(StreamHttpError),
}

// The lists a failed round reads, plus the translation that turns the failure into stream output
struct FailureContext<'context, T: ?Sized> {
    translation: &'context T,
    call_type: CallType,
    responses_so_far: &'context [GuardrailValue],
    responses_yielded: &'context [GuardrailValue],
}

pub(crate) async fn emit_transform_round<T, G>(
    params: &mut StreamLoopParams<'_, T, G>,
    state: &mut TransformState,
    buffers: StreamBuffers<'_>,
    reference_chunk: &GuardrailValue,
    is_final: bool,
) -> Result<(), RoundFailure>
where
    T: BaseTranslation<Guardrail = G, LogTarget = GuardrailLogTarget, Failure = GuardrailFailure>
        + Sync,
    G: GuardrailContract,
{
    let StreamBuffers {
        responses_so_far,
        responses_yielded,
        outputs,
    } = buffers;
    let called = run_round_call(params, state, responses_so_far, reference_chunk, is_final).await;
    let synthetic = match called {
        Ok(synthetic) => synthetic,
        Err(failure) => {
            let context = FailureContext {
                translation: params.translation,
                call_type: params.call_type,
                responses_so_far: responses_so_far.as_slice(),
                responses_yielded: responses_yielded.as_slice(),
            };
            return handle_round_failure(failure, &context, &*params.request_data, outputs);
        }
    };
    if let Some(chunk) = synthetic {
        responses_yielded.push(chunk.clone());
        outputs.push_back(StreamOutput::Item(chunk));
    }
    Ok(())
}

// LiteLLM's process_output_streaming_response call, shared with the block-only inspection
pub(crate) async fn call_streaming<T, G>(
    params: &mut StreamLoopParams<'_, T, G>,
    responses_so_far: &mut Vec<GuardrailValue>,
    sink: Option<&mut StreamTransformSink>,
) -> Result<(), GuardrailFailure>
where
    T: BaseTranslation<Guardrail = G, LogTarget = GuardrailLogTarget, Failure = GuardrailFailure>
        + Sync,
    G: GuardrailContract,
{
    let mut target = request_log_target(&*params.request_data);
    let call = TranslationCall {
        user_api_key_dict: Some(params.user_api_key_dict),
        request_data: Some(&mut *params.request_data),
        logging_obj: target.as_mut(),
        stream_transform_sink: sink,
        deliver_ended_stream_rewrites: T::DELIVERS_ENDED_STREAM_REWRITES,
    };
    params
        .translation
        .process_streaming_response(responses_so_far, params.guardrail, call)
        .await
}

async fn run_round_call<T, G>(
    params: &mut StreamLoopParams<'_, T, G>,
    state: &mut TransformState,
    responses_so_far: &mut Vec<GuardrailValue>,
    reference_chunk: &GuardrailValue,
    is_final: bool,
) -> Result<Option<GuardrailValue>, RoundCallError>
where
    T: BaseTranslation<Guardrail = G, LogTarget = GuardrailLogTarget, Failure = GuardrailFailure>
        + Sync,
    G: GuardrailContract,
{
    let mut sink = StreamTransformSink::default();
    call_streaming(params, responses_so_far, Some(&mut sink))
        .await
        .map_err(RoundCallError::Guardrail)?;
    build_transform_chunk(TransformChunkInput {
        reference_chunk,
        mutated_text_per_choice: &sink.mutated_text_per_choice,
        emitted_text_per_choice: &mut state.emitted_text,
        holdback_per_choice: &sink.holdback_per_choice,
        finish_reason_per_choice: &state.finish_reasons,
        held_chars_per_choice: &mut state.held_chars,
        is_final,
    })
    .map_err(RoundCallError::Http)
}

fn handle_round_failure<T>(
    failure: RoundCallError,
    context: &FailureContext<'_, T>,
    request_data: &BTreeMap<String, GuardrailValue>,
    outputs: &mut VecDeque<StreamOutput>,
) -> Result<(), RoundFailure>
where
    T: BaseTranslation<Failure = GuardrailFailure> + ?Sized,
{
    match failure {
        RoundCallError::Http(error) => terminate_with_http(error, context, request_data, outputs),
        RoundCallError::Guardrail(GuardrailFailure::ModifyResponse(mut details)) => {
            if details.original_response.is_none() {
                details.original_response =
                    Some(GuardrailValue::Array(context.responses_so_far.to_vec()));
            }
            terminate_with_block(
                context.translation,
                GuardrailFailure::ModifyResponse(details),
                context.responses_yielded,
                outputs,
            )
        }
        RoundCallError::Guardrail(other) => {
            let Some(error) = http_failure(&other) else {
                return Err(RoundFailure::Loop(StreamLoopError::Guardrail {
                    source: other,
                }));
            };
            terminate_with_http(error, context, request_data, outputs)
        }
    }
}

// LiteLLM turns an HTTPException inside a round into an in-stream error and stops the stream
fn terminate_with_http<T>(
    error: StreamHttpError,
    context: &FailureContext<'_, T>,
    request_data: &BTreeMap<String, GuardrailValue>,
    outputs: &mut VecDeque<StreamOutput>,
) -> Result<(), RoundFailure>
where
    T: BaseTranslation<Failure = GuardrailFailure> + ?Sized,
{
    let emission = StreamErrorEmission {
        call_type: Some(context.call_type),
        responses_so_far: context.responses_so_far,
        request_data,
        translation: Some(context.translation),
        stream_started: !context.responses_yielded.is_empty(),
        responses_yielded: Some(context.responses_yielded),
    };
    let items = emit_streaming_http_error(error, &emission)
        .map_err(|source| StreamLoopError::Http { source })?;
    for item in items {
        outputs.push_back(StreamOutput::Item(item));
    }
    Err(RoundFailure::Terminated(StreamTerminated))
}

// LiteLLM yields the block message and returns; a format without one re-raises the failure
pub(crate) fn terminate_with_block<T>(
    translation: &T,
    failure: GuardrailFailure,
    responses_yielded: &[GuardrailValue],
    outputs: &mut VecDeque<StreamOutput>,
) -> Result<(), RoundFailure>
where
    T: BaseTranslation<Failure = GuardrailFailure> + ?Sized,
{
    let chunks = handle_streaming_block(
        translation,
        failure,
        !responses_yielded.is_empty(),
        responses_yielded,
    )
    .map_err(|source| StreamLoopError::Block { source })?;
    outputs.push_back(StreamOutput::BlockChunks(chunks));
    Err(RoundFailure::Terminated(StreamTerminated))
}

fn http_failure(failure: &GuardrailFailure) -> Option<StreamHttpError> {
    match failure {
        GuardrailFailure::Http { status, detail } => Some(StreamHttpError {
            status_code: *status,
            detail: HttpErrorDetail::Text(detail.clone()),
        }),
        GuardrailFailure::HttpObject { status, detail } => Some(StreamHttpError {
            status_code: *status,
            detail: HttpErrorDetail::Object(
                detail
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
        }),
        GuardrailFailure::ModifyResponse(_)
        | GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Other(_) => None,
    }
}

// LiteLLM reads request_data.get("litellm_logging_obj") and passes it to the translation
pub(crate) fn request_log_target(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Option<GuardrailLogTarget> {
    let GuardrailValue::Object(logging) = request_data.get("litellm_logging_obj")? else {
        return None;
    };
    let litellm_params = object_field(logging, "litellm_params");
    let model_call_params = object_field(logging, "model_call_details")
        .and_then(|details| object_field(&details, "litellm_params"));
    Some(GuardrailLogTarget {
        litellm_params,
        model_call_params,
    })
}

fn object_field(
    fields: &BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Option<BTreeMap<String, GuardrailValue>> {
    match fields.get(key) {
        Some(GuardrailValue::Object(inner)) => Some(inner.clone()),
        _ => None,
    }
}

mod tests;
