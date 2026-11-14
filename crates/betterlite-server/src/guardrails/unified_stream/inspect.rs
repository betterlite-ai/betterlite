use std::collections::VecDeque;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::BaseTranslation;

use super::error::{RoundFailure, StreamLoopError};
use super::round;
use super::{StreamLoopParams, StreamOutput};

// LiteLLM's _inspect_full_response_for_block: the transforms are dropped, only the block matters
pub(crate) async fn inspect_response_for_block<T, G, E>(
    params: &mut StreamLoopParams<'_, T, G>,
    assembled: &mut Vec<GuardrailValue>,
    responses_yielded: &[GuardrailValue],
    outputs: &mut VecDeque<StreamOutput>,
) -> Result<(), RoundFailure<E>>
where
    T: BaseTranslation<Guardrail = G, LogTarget = GuardrailLogTarget, Failure = GuardrailFailure>
        + Sync,
    G: GuardrailContract,
{
    let called = round::call_streaming(params, &mut *assembled, None).await;
    let failure = match called {
        Ok(()) => return Ok(()),
        Err(failure) => failure,
    };
    match failure {
        GuardrailFailure::ModifyResponse(mut details) => {
            if details.original_response.is_none() {
                details.original_response = Some(GuardrailValue::Array(assembled.clone()));
            }
            round::terminate_with_block(
                params.translation,
                GuardrailFailure::ModifyResponse(details),
                responses_yielded,
                outputs,
            )
        }
        other => Err(RoundFailure::Loop(StreamLoopError::Guardrail {
            source: other,
        })),
    }
}

mod tests;
