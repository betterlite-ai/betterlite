use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_message_scope::{Object, StreamItem};

use super::stream_request::{stream_inputs, stream_request_data};
use crate::guardrails::chat_guardrail_translation::extraction::combine_texts::{
    TextKey, combine_streaming_texts,
};
use crate::guardrails::chat_guardrail_translation::extraction::first_choice_finished::first_choice_has_finished;
use crate::guardrails::chat_guardrail_translation::extraction::has_text_content::has_text_content;
use crate::guardrails::chat_guardrail_translation::rewrites::ended_stream_gather::{
    EndedStream, process_ended_stream,
};
use crate::guardrails::chat_guardrail_translation::rewrites::error::EndedStreamError;
use crate::guardrails::chat_guardrail_translation::rewrites::rebuild_ended_stream::ChunkAssembler;
use crate::guardrails::chat_guardrail_translation::rewrites::write_text_rewrites::streaming::apply_output_streaming;

mod tests;

// A finished stream is rebuilt and scanned whole; a running one is scanned as the text so far, so a block can end it.
pub async fn process_streaming_block_only<G, A>(
    responses_so_far: &mut [Object],
    guardrail: &G,
    assembler: &A,
    ended: &EndedStream<'_>,
) -> Result<(), EndedStreamError>
where
    G: GuardrailContract,
    A: ChunkAssembler,
{
    let stream_items: Vec<StreamItem> = responses_so_far
        .iter()
        .cloned()
        .map(|chunk| StreamItem::Json(GuardrailValue::Object(chunk)))
        .collect();
    if first_choice_has_finished(&stream_items) {
        return process_ended_stream(responses_so_far, guardrail, assembler, ended).await;
    }
    if !responses_so_far.iter().any(has_text_content) {
        tracing::warn!(
            "OpenAI Chat Completions: No text content in streaming responses, skipping guardrail"
        );
        return Ok(());
    }
    let (mappings, texts): (Vec<TextKey>, Vec<String>) = combine_streaming_texts(responses_so_far)
        .into_iter()
        .unzip();
    if texts.is_empty() {
        return Ok(());
    }
    let mut request_data = stream_request_data(responses_so_far, ended.principal);
    let inputs = stream_inputs(texts, responses_so_far);
    let guardrailed = guardrail
        .apply_guardrail(
            inputs,
            &mut request_data,
            GuardrailInputType::Response,
            None,
        )
        .await?;
    let guardrailed_texts = guardrailed.texts.unwrap_or_default();
    apply_output_streaming(responses_so_far, &guardrailed_texts, &mappings)?;
    Ok(())
}
