use std::collections::BTreeMap;
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, StreamingScanKey, TranslationCall,
};
use thiserror::Error;

use super::stamps::StampSource;
use super::{GuardrailToApply, failure_message, modify_response};
use crate::guardrails::anthropic_translation::block_continuation::{self, BlockedException};
use crate::guardrails::anthropic_translation::output_scan;
use crate::guardrails::anthropic_translation::output_scan::error::ProcessOutputError;
use crate::guardrails::anthropic_translation::post_call_hook;
use crate::guardrails::anthropic_translation::process_input;
use crate::guardrails::anthropic_translation::process_input::InputContext;
use crate::guardrails::anthropic_translation::process_input::error::ProcessInputError;
use crate::guardrails::anthropic_translation::process_stream;
use crate::guardrails::anthropic_translation::process_stream::error::ProcessStreamError;
use crate::guardrails::anthropic_translation::sse_events::{self, SseChunk};
use crate::guardrails::responses_translation::block_events::guardrail_json::{
    blocked_original_response, to_guardrail,
};

#[derive(Debug, Error)]
pub enum AnthropicTranslationError {
    #[error(transparent)]
    Input(#[from] ProcessInputError),
    #[error(transparent)]
    Output(#[from] ProcessOutputError),
    #[error(transparent)]
    Stream(#[from] ProcessStreamError),
}

impl AnthropicTranslationError {
    fn guardrail(&self) -> Option<&GuardrailFailure> {
        match self {
            Self::Input(ProcessInputError::Guardrail { source })
            | Self::Output(ProcessOutputError::Guardrail { source }) => Some(source.as_ref()),
            Self::Stream(ProcessStreamError::Guardrail(failure)) => Some(failure),
            Self::Input(_) | Self::Output(_) | Self::Stream(_) => None,
        }
    }
}

pub struct AnthropicTranslation<G, S> {
    pub(crate) stamps: S,
    pub(crate) _guardrail: PhantomData<fn() -> G>,
}

impl<G, S> BaseTranslation for AnthropicTranslation<G, S>
where
    G: GuardrailContract + Send + Sync,
    S: StampSource + Send + Sync,
{
    type Guardrail = GuardrailToApply<G>;
    type LogTarget = GuardrailLogTarget;
    type Failure = AnthropicTranslationError;

    const DELIVERS_ENDED_STREAM_REWRITES: bool = true;
    const ASSEMBLES_STREAMED_RESPONSE: bool = true;

    fn post_call_hook_response(&self, response: GuardrailValue) -> GuardrailValue {
        match post_call_hook::post_call_hook_response(&response) {
            Ok(translated) => translated,
            Err(error) => {
                tracing::error!(
                    "Anthropic guardrail: the post-call hook left the response untranslated: {error}"
                );
                response
            }
        }
    }

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let context = InputContext {
            skip_flags: guardrail.skip_flags,
            global_flags: guardrail.global_flags,
            guardrail_name: guardrail.guardrail_name.as_deref(),
        };
        process_input::process_input_messages(
            data,
            guardrail.guardrail.as_ref(),
            &context,
            logging_obj,
        )
        .await?;
        Ok(())
    }

    async fn process_output_response(
        &self,
        mut response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        output_scan::process_output_response(
            &mut response,
            guardrail.guardrail.as_ref(),
            call.logging_obj,
            call.user_api_key_dict,
            call.request_data,
        )
        .await?;
        Ok(response)
    }

    async fn process_streaming_response(
        &self,
        responses_so_far: &mut Vec<StreamItem>,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let mut chunks = responses_so_far
            .iter()
            .map(sse_events::stream_item_chunk)
            .collect::<Result<Vec<SseChunk>, serde_json::Error>>()
            .map_err(ProcessStreamError::Convert)?;
        let name = guardrail.guardrail_name.as_deref();
        let applied = guardrail.guardrail.as_ref();
        let logged = call.logging_obj;
        let user_api_key_dict = call.user_api_key_dict;
        let request_data = call.request_data;
        let deliver = call.deliver_ended_stream_rewrites;
        // ast-grep-ignore: no-long-names
        let result = process_stream::process_output_streaming_response(
            &mut chunks,
            applied,
            name,
            logged,
            user_api_key_dict,
            request_data,
            deliver,
        )
        .await;
        write_chunks(responses_so_far, &chunks);
        result?;
        Ok(())
    }

    fn get_streaming_scan_key(&self, responses_so_far: &[StreamItem]) -> Option<StreamingScanKey> {
        let mut chunks = Vec::with_capacity(responses_so_far.len());
        for item in responses_so_far {
            chunks.push(sse_events::stream_item_chunk(item).ok()?);
        }
        Some(sse_events::streaming_scan_key(&chunks))
    }

    fn build_block_sse_chunks(
        &self,
        failure: &Self::Failure,
        stream_started: bool,
        responses_so_far: Option<&[StreamItem]>,
    ) -> Option<Vec<Vec<u8>>> {
        let details = modify_response(failure.guardrail()?)?;
        let stamp = self.stamps.stamp();
        let original = serde_json::to_value(details.original_response.as_ref()).ok()?;
        let original_response = blocked_original_response(Some(&original));
        let chunks = responses_so_far
            .map(|items| {
                items
                    .iter()
                    .map(sse_events::stream_item_chunk)
                    .collect::<Result<Vec<SseChunk>, serde_json::Error>>()
            })
            .transpose()
            .ok()?;
        let blocked = BlockedException {
            message: &details.message,
            model: &details.model,
            message_id: &stamp.message_id,
            original_response: &original_response,
        };
        block_continuation::build_block_sse_chunks(&blocked, stream_started, chunks.as_deref()).ok()
    }

    fn build_stream_error_items(
        &self,
        failure: &Self::Failure,
        _responses_so_far: Option<&[StreamItem]>,
    ) -> Option<Vec<StreamItem>> {
        let (_, message) = failure_message(failure.guardrail()?)?;
        let frames = block_continuation::build_stream_error_items(&message).ok()?;
        Some(frames.into_iter().map(StreamItem::Bytes).collect())
    }
}

fn write_chunks(items: &mut [StreamItem], chunks: &[SseChunk]) {
    for (slot, chunk) in items.iter_mut().zip(chunks) {
        match chunk {
            SseChunk::Event(fields) => {
                *slot = StreamItem::Json(GuardrailValue::Object(
                    fields
                        .iter()
                        .map(|(key, value)| (key.clone(), to_guardrail(value)))
                        .collect(),
                ));
            }
            SseChunk::Bytes(bytes) => {
                *slot = StreamItem::Bytes(bytes.clone());
            }
            SseChunk::Other => {}
        }
    }
}

mod tests;
