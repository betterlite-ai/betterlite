use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::BaseTranslation;
use betterlite_guardrails::unified_dispatch::stream_items::trailing_metadata_chunks;

use super::error::RoundFailure;
use super::round::{self, TransformState};
use super::{StreamBuffers, StreamLoopParams, StreamOutput};

// LiteLLM's _emit_stream_tail: flush the held text with holdback 0, then replay metadata chunks
pub(crate) async fn emit_stream_tail<T, G>(
    params: &mut StreamLoopParams<'_, T, G>,
    state: &mut TransformState,
    buffers: StreamBuffers<'_>,
    last_chunk: Option<&GuardrailValue>,
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
    if let Some(reference_chunk) = last_chunk {
        round::emit_transform_round(
            params,
            state,
            StreamBuffers {
                responses_so_far: &mut *responses_so_far,
                responses_yielded: &mut *responses_yielded,
                outputs: &mut *outputs,
            },
            reference_chunk,
            true,
        )
        .await?;
    }
    for trailing in trailing_metadata_chunks(responses_so_far.as_slice()) {
        let trailing = trailing.clone();
        responses_yielded.push(trailing.clone());
        outputs.push_back(StreamOutput::Item(trailing));
    }
    Ok(())
}

mod tests;
