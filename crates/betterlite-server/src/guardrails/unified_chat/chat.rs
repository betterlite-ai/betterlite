use std::collections::BTreeMap;
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailMessage;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::ScopeFlags;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, StreamingScanKey, TranslationCall,
};
use thiserror::Error;

use self::support::{
    failure_error_object, fresh_identity, object_chunks, write_back, write_object_chunks,
};
use crate::guardrails::chat_guardrail_translation::block_framing::block_sse_chunks::{
    self, BlockedResponse,
};
use crate::guardrails::chat_guardrail_translation::extraction::error_items;
use crate::guardrails::chat_guardrail_translation::extraction::extract_tool_names;
use crate::guardrails::chat_guardrail_translation::rewrites as chat_rewrites;
use crate::guardrails::chat_guardrail_translation::rewrites::ended_stream_gather::EndedStream;
use crate::guardrails::chat_guardrail_translation::rewrites::error::{
    EndedStreamError, InputError,
};
use crate::guardrails::chat_guardrail_translation::rewrites::process_input::InputScope;
use crate::guardrails::chat_guardrail_translation::rewrites::rebuild_ended_stream::ChunkAssembler;
use crate::guardrails::chat_guardrail_translation::rewrites::response::ChatResponse;
use crate::guardrails::chat_guardrail_translation::rewrites::structured::guardrail_messages;
use crate::guardrails::chat_guardrail_translation::streaming::process_output_streaming;
use crate::guardrails::chat_guardrail_translation::streaming::streaming_scan_key;
use crate::guardrails::responses_translation::block_events::guardrail_json::blocked_original_response;

use super::stamps::StampSource;
use super::{GuardrailToApply, UNKNOWN_GUARDRAIL_NAME, modify_response};

#[derive(Debug, Error)]
pub enum ChatTranslationError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error(transparent)]
    Input(#[from] InputError),
    #[error(transparent)]
    Stream(#[from] EndedStreamError),
    #[error("the chat response could not be read: {source}")]
    Parse {
        #[from]
        source: serde_json::Error,
    },
}

impl ChatTranslationError {
    const fn guardrail(&self) -> Option<&GuardrailFailure> {
        match self {
            Self::Guardrail(failure)
            | Self::Input(InputError::Guardrail(failure))
            | Self::Stream(EndedStreamError::Guardrail(failure)) => Some(failure),
            Self::Input(_) | Self::Stream(_) | Self::Parse { .. } => None,
        }
    }
}

pub struct ChatTranslation<G, S, A> {
    pub(crate) stamps: S,
    pub(crate) assembler: A,
    pub(crate) _guardrail: PhantomData<fn() -> G>,
}

impl<G, S, A> BaseTranslation for ChatTranslation<G, S, A>
where
    G: GuardrailContract + Send + Sync,
    S: StampSource + Send + Sync,
    A: ChunkAssembler + Send + Sync,
{
    type Guardrail = GuardrailToApply<G>;
    type LogTarget = GuardrailLogTarget;
    type Failure = ChatTranslationError;

    const DELIVERS_ENDED_STREAM_REWRITES: bool = true;
    const ASSEMBLES_STREAMED_RESPONSE: bool = true;

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let scope = InputScope {
            flags: ScopeFlags::resolve(&guardrail.skip_flags, &guardrail.global_flags),
            guardrail_name: guardrail.guardrail_name.as_deref(),
        };
        let applied = guardrail.guardrail.as_ref();
        let _skip_reason = chat_rewrites::process_input::process_input_messages(
            data,
            applied,
            &scope,
            logging_obj,
        )
        .await?;
        Ok(())
    }

    async fn process_output_response(
        &self,
        mut response: GuardrailValue,
        guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        let mut parsed: ChatResponse = serde_json::from_value(serde_json::to_value(&response)?)?;
        let before = parsed.clone();
        chat_rewrites::process_output_response(
            &mut parsed,
            guardrail.guardrail.as_ref(),
            &guardrail.principal,
        )
        .await?;
        write_back(&mut response, &before, &parsed);
        Ok(response)
    }

    async fn process_streaming_response(
        &self,
        responses_so_far: &mut Vec<StreamItem>,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let (positions, mut chunks) = object_chunks(responses_so_far);
        let ended = EndedStream {
            principal: &guardrail.principal,
            guardrail_name: guardrail
                .guardrail_name
                .as_deref()
                .unwrap_or(UNKNOWN_GUARDRAIL_NAME),
            deliver_rewrites: call.deliver_ended_stream_rewrites,
        };
        let sink = call.stream_transform_sink;
        let assembler = &self.assembler;
        let applied = guardrail.guardrail.as_ref();
        process_output_streaming::process_streaming_response(
            &mut chunks,
            applied,
            assembler,
            &ended,
            sink,
        )
        .await?;
        write_object_chunks(responses_so_far, &positions, &chunks);
        Ok(())
    }

    fn get_streaming_scan_key(&self, responses_so_far: &[StreamItem]) -> Option<StreamingScanKey> {
        streaming_scan_key::get_streaming_scan_key(responses_so_far).ok()
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
        let blocked = BlockedResponse {
            message: details.message.clone(),
            model: details.model.clone(),
            original_response: blocked_original_response(Some(&original)),
        };
        let (_, chunks) = object_chunks(responses_so_far.unwrap_or(&[]));
        let items: Vec<StreamItem> = chunks
            .into_iter()
            .map(|chunk| StreamItem::Json(GuardrailValue::Object(chunk)))
            .collect();
        let frames = block_sse_chunks::build_block_sse_chunks(
            &blocked,
            stream_started,
            &items,
            &fresh_identity(&stamp),
        )
        .ok()?;
        Some(Vec::from(frames))
    }

    fn build_stream_error_items(
        &self,
        failure: &Self::Failure,
        _responses_so_far: Option<&[StreamItem]>,
    ) -> Option<Vec<StreamItem>> {
        let error = failure_error_object(failure.guardrail()?)?;
        let frames = error_items::build_stream_error_items(&error).ok()?;
        Some(frames.into_iter().map(StreamItem::Bytes).collect())
    }

    fn get_structured_messages(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
    ) -> Option<Vec<GuardrailMessage>> {
        let Some(GuardrailValue::Array(messages)) = data.get("messages") else {
            return None;
        };
        let scoped: Vec<usize> = (0..messages.len()).collect();
        guardrail_messages(messages, &scoped).ok()
    }

    fn extract_request_tool_names(&self, data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        extract_tool_names::extract_request_tool_names(data)
    }
}

mod support;
mod tests;
