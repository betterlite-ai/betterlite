use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_translation::StreamTransformSink;

use super::block_only::process_streaming_block_only;
use super::streaming_transform::process_streaming_transform;
use crate::guardrails::chat_guardrail_translation::rewrites::ended_stream_gather::EndedStream;
use crate::guardrails::chat_guardrail_translation::rewrites::error::EndedStreamError;
use crate::guardrails::chat_guardrail_translation::rewrites::rebuild_ended_stream::ChunkAssembler;

mod tests;

// A sink asks for the transform path, which only reads the chunks; without one the block-only path runs and may write rewrites into them.
pub async fn process_streaming_response<G, A>(
    responses_so_far: &mut [Object],
    guardrail: &G,
    assembler: &A,
    ended: &EndedStream<'_>,
    sink: Option<&mut StreamTransformSink>,
) -> Result<(), EndedStreamError>
where
    G: GuardrailContract,
    A: ChunkAssembler,
{
    if let Some(transform_sink) = sink {
        process_streaming_transform(responses_so_far, guardrail, ended.principal, transform_sink)
            .await?;
        return Ok(());
    }
    process_streaming_block_only(responses_so_far, guardrail, assembler, ended).await
}
