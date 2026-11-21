use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, StreamingScanKey, TranslationCall,
};
use thiserror::Error;

use super::stamps::StampSource;
use super::{GuardrailToApply, failure_message, frames_into_items, modify_response, to_json};
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
            Self::Input(ProcessInputError::Guardrail { source }) => Some(source.as_ref()),
            Self::Output(ProcessOutputError::Guardrail { source }) => Some(source.as_ref()),
            Self::Stream(ProcessStreamError::Guardrail(failure)) => Some(failure),
            Self::Input(_) | Self::Output(_) | Self::Stream(_) => None,
        }
    }
}

pub struct AnthropicTranslation<G, S> {
    stamps: S,
    _guardrail: PhantomData<fn() -> G>,
}

impl<G, S> AnthropicTranslation<G, S> {
    pub fn new(stamps: S) -> Self {
        Self {
            stamps,
            _guardrail: PhantomData,
        }
    }
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

    fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        let context = InputContext {
            skip_flags: guardrail.skip_flags,
            global_flags: guardrail.global_flags,
            guardrail_name: guardrail.guardrail_name.as_deref(),
        };
        let applied = guardrail.guardrail.as_ref();
        async move {
            process_input::process_input_messages(data, applied, &context, logging_obj).await?;
            Ok(())
        }
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

    fn process_streaming_response(
        &self,
        responses_so_far: &mut Vec<GuardrailValue>,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        let mut chunks = to_chunks(responses_so_far);
        let name = guardrail.guardrail_name.as_deref();
        let applied = guardrail.guardrail.as_ref();
        let logged = call.logging_obj;
        let user_api_key_dict = call.user_api_key_dict;
        let request_data = call.request_data;
        let deliver = call.deliver_ended_stream_rewrites;
        async move {
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
    }

    fn get_streaming_scan_key(
        &self,
        responses_so_far: &[GuardrailValue],
    ) -> Option<StreamingScanKey> {
        Some(sse_events::streaming_scan_key(&to_chunks(responses_so_far)))
    }

    fn build_block_sse_chunks(
        &self,
        failure: &Self::Failure,
        stream_started: bool,
        responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<Vec<u8>>> {
        let details = modify_response(failure.guardrail()?)?;
        let stamp = self.stamps.stamp();
        let original = details.original_response.as_ref().map(to_json);
        let original_response = blocked_original_response(original.as_ref());
        let chunks = responses_so_far.map(to_chunks);
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
        _responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<GuardrailValue>> {
        let (_, message) = failure_message(failure.guardrail()?)?;
        block_continuation::build_stream_error_items(&message)
            .ok()
            .and_then(frames_into_items)
    }
}

// The chat handler needs every chunk, so a value that is not an object stays a chunk B3676 leaves alone
fn to_chunks(items: &[GuardrailValue]) -> Vec<SseChunk> {
    items
        .iter()
        .map(|item| match item {
            GuardrailValue::Object(fields) => SseChunk::Event(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), to_json(value)))
                    .collect(),
            ),
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_) => SseChunk::Other,
        })
        .collect()
}

fn write_chunks(items: &mut [GuardrailValue], chunks: &[SseChunk]) {
    for (slot, chunk) in items.iter_mut().zip(chunks) {
        let SseChunk::Event(fields) = chunk else {
            continue;
        };
        *slot = GuardrailValue::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), to_guardrail(value)))
                .collect(),
        );
    }
}

mod tests;
