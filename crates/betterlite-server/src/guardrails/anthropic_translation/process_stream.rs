#![expect(
    clippy::disallowed_types,
    clippy::too_many_arguments,
    reason = "the buffered Anthropic chunks are provider JSON that betterlite does not model, and the entry point keeps LiteLLM's argument list"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        reason = "the tests compare provider JSON numbers with plain integer literals"
    )
)]

mod assembly;
mod convert;
mod ended;
pub mod error;
mod prepare;
mod tests;
mod unended;

use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::Object;

use self::error::ProcessStreamError;
use crate::guardrails::anthropic_translation::sse_events::{SseChunk, streaming_has_ended};

struct StreamContext<'ctx> {
    guardrail_name: &'ctx str,
    user_api_key_dict: Option<&'ctx Object>,
    deliver: bool,
}

// LiteLLM's process_output_streaming_response: a stream that reported a stop_reason is assembled and scanned whole, and the buffered chunks take the rewrite.
// ast-grep-ignore: no-long-names
pub async fn process_output_streaming_response<G>(
    responses_so_far: &mut Vec<SseChunk>,
    guardrail: &G,
    guardrail_name: Option<&str>,
    logging_obj: Option<&mut GuardrailLogTarget>,
    user_api_key_dict: Option<&Object>,
    request_data: Option<&mut Object>,
    deliver_ended_stream_rewrites: bool,
) -> Result<(), ProcessStreamError>
where
    G: GuardrailContract,
{
    let context = StreamContext {
        guardrail_name: guardrail_name.unwrap_or("unknown"),
        user_api_key_dict,
        deliver: deliver_ended_stream_rewrites,
    };
    if streaming_has_ended(responses_so_far) {
        return ended::process_ended(
            responses_so_far,
            guardrail,
            &context,
            logging_obj,
            request_data,
        )
        .await;
    }
    unended::process_unended(
        responses_so_far,
        guardrail,
        &context,
        logging_obj,
        request_data,
    )
    .await
}
