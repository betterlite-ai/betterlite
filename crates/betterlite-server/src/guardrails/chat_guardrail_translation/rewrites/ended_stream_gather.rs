use betterlite_auth::auth_identity::records::Principal;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::error::EndedStreamError;
use super::process_output_response;
use super::rebuild_ended_stream::{ChunkAssembler, rebuild_per_choice};
use super::string_choice_contents::string_choice_contents;
use super::tool_call_shapes::function_tool_call_shapes;
use super::write_text_rewrites::write_ended_text_rewrites;
use super::write_tool_rewrites::write_ended_tool_rewrites;

mod tests;

pub struct EndedStream<'stream> {
    pub principal: &'stream Principal,
    pub guardrail_name: &'stream str,
    pub deliver_rewrites: bool,
}

// A tool-call rewrite the stream cannot carry comes back as Undeliverable, after any text rewrite is already written, so the caller keeps the original chunks to release instead.
pub async fn process_ended_stream<G, A>(
    responses_so_far: &mut [Object],
    guardrail: &G,
    assembler: &A,
    ended: &EndedStream<'_>,
) -> Result<(), EndedStreamError>
where
    G: GuardrailContract,
    A: ChunkAssembler,
{
    let mut model_response = rebuild_per_choice(responses_so_far, assembler).await?;
    let pre_guardrail_texts = string_choice_contents(&model_response);
    let pre_guardrail_tool_calls = function_tool_call_shapes(&model_response);
    process_output_response(&mut model_response, guardrail, ended.principal).await?;
    if !ended.deliver_rewrites {
        return Ok(());
    }
    write_ended_text_rewrites(responses_so_far, &model_response, &pre_guardrail_texts)?;
    write_ended_tool_rewrites(
        responses_so_far,
        &model_response,
        &pre_guardrail_tool_calls,
        ended.guardrail_name,
    )?;
    Ok(())
}
