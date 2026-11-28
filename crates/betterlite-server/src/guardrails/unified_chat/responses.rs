use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::settings::JsonValue;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::guardrails::streaming_rewrite::failure::UndeliverableStreamRewrite;
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

use super::stamps::StampSource;
use super::undeliverable;
use super::{GuardrailToApply, failure_message, modify_response};
use crate::guardrails::responses_translation::block_events;
use crate::guardrails::responses_translation::block_events::guardrail_json::to_guardrail;
use crate::guardrails::responses_translation::output_rewrite;
use crate::guardrails::responses_translation::output_rewrite::OutputContext;
use crate::guardrails::responses_translation::output_rewrite::error::OutputRewriteError;
use crate::guardrails::responses_translation::request_rewrite;
use crate::guardrails::responses_translation::request_rewrite::RequestContext;
use crate::guardrails::responses_translation::request_rewrite::error::RequestRewriteError;
use crate::guardrails::responses_translation::stream_output;
use crate::guardrails::responses_translation::stream_output::error::StreamOutputError;
use crate::guardrails::responses_translation::stream_scan;

#[derive(Debug, Error)]
pub enum ResponsesTranslationError {
    #[error(transparent)]
    Request(#[from] RequestRewriteError),
    #[error(transparent)]
    Output(#[from] OutputRewriteError),
    #[error(transparent)]
    Stream(#[from] StreamOutputError),
    #[error(transparent)]
    Undeliverable(#[from] UndeliverableStreamRewrite),
    #[error("the guardrail data could not be read as JSON")]
    Json(#[from] serde_json::Error),
}

impl ResponsesTranslationError {
    fn guardrail(&self) -> Option<&GuardrailFailure> {
        match self {
            Self::Request(RequestRewriteError::Guardrail { source })
            | Self::Output(OutputRewriteError::Guardrail { source }) => Some(source.as_ref()),
            Self::Stream(StreamOutputError::Guardrail(failure)) => Some(failure),
            Self::Request(_)
            | Self::Output(_)
            | Self::Stream(_)
            | Self::Undeliverable(_)
            | Self::Json(_) => None,
        }
    }
}

pub struct ResponsesTranslation<G, S> {
    pub(crate) stamps: S,
    pub(crate) limits: ApiLimits,
    pub(crate) cache: Arc<DualLocalCache>,
    pub(crate) _guardrail: PhantomData<fn() -> G>,
}

impl<G, S> BaseTranslation for ResponsesTranslation<G, S>
where
    G: GuardrailContract + Send + Sync,
    S: StampSource + Send + Sync,
{
    type Guardrail = GuardrailToApply<G>;
    type LogTarget = GuardrailLogTarget;
    type Failure = ResponsesTranslationError;

    const DELIVERS_ENDED_STREAM_REWRITES: bool = true;
    const ASSEMBLES_STREAMED_RESPONSE: bool = true;

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let context = RequestContext {
            skip_system_message: ScopeFlags::resolve(
                &guardrail.skip_flags,
                &guardrail.global_flags,
            )
            .skip_system_message,
            guardrail_name: guardrail.guardrail_name.as_deref(),
            limits: self.limits,
            cache: self.cache.as_ref(),
        };
        request_rewrite::process_input_messages(
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
        let context = OutputContext {
            guardrail_name: guardrail.guardrail_name.as_deref(),
            user_api_key_dict: call.user_api_key_dict,
        };
        output_rewrite::process_output_response(
            &mut response,
            guardrail.guardrail.as_ref(),
            &context,
            call.logging_obj,
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
        let mut events = events_of(responses_so_far)?;
        let name = guardrail.guardrail_name.as_deref();
        let applied = guardrail.guardrail.as_ref();
        let logged = call.logging_obj;
        let user_api_key_dict = call.user_api_key_dict;
        let request_data = call.request_data;
        let deliver = call.deliver_ended_stream_rewrites;
        // ast-grep-ignore: no-long-names
        let result = stream_output::process_output_streaming_response(
            &mut events,
            applied,
            name,
            logged,
            user_api_key_dict,
            request_data,
            deliver,
        )
        .await;
        write_events(responses_so_far, &events);
        match result {
            Ok(()) => Ok(()),
            Err(StreamOutputError::Sync(sync)) => Err(ResponsesTranslationError::from(
                undeliverable::from_stream_sync(sync),
            )),
            Err(error) => Err(ResponsesTranslationError::Stream(error)),
        }
    }

    fn get_streaming_scan_key(&self, responses_so_far: &[StreamItem]) -> Option<StreamingScanKey> {
        let events = events_of(responses_so_far).ok()?;
        stream_scan::streaming_scan_key(&events)
    }

    fn build_block_sse_chunks(
        &self,
        failure: &Self::Failure,
        stream_started: bool,
        responses_so_far: Option<&[StreamItem]>,
    ) -> Option<Vec<Vec<u8>>> {
        let details = modify_response(failure.guardrail()?)?;
        let original = serde_json::to_value(details.original_response.as_ref()).ok()?;
        let events = responses_so_far.map(events_of).transpose().ok()?;
        block_events::build_block_sse_chunks(
            &details.message,
            &details.model,
            Some(&original),
            stream_started,
            events.as_deref(),
            &self.stamps.stamp(),
        )
        .ok()
    }

    fn build_stream_error_items(
        &self,
        failure: &Self::Failure,
        responses_so_far: Option<&[StreamItem]>,
    ) -> Option<Vec<StreamItem>> {
        let (status, message) = failure_message(failure.guardrail()?)?;
        let events = responses_so_far.map(events_of).transpose().ok()?;
        block_events::build_stream_error_items(status, &message, events.as_deref())
            .into_iter()
            .map(|event| {
                serde_json::to_value(event)
                    .ok()
                    .map(|value| StreamItem::Json(to_guardrail(&value)))
            })
            .collect()
    }

    fn get_structured_messages(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
    ) -> Option<Vec<GuardrailMessage>> {
        let context = RequestContext {
            skip_system_message: false,
            guardrail_name: None,
            limits: self.limits,
            cache: self.cache.as_ref(),
        };
        let messages = request_rewrite::get_structured_messages(data, &context).ok()??;
        messages
            .into_iter()
            .map(|message| serde_json::from_value(message).ok())
            .collect()
    }
}

// A raw frame keeps its slot as null, since the events after it must keep their positions
fn events_of(items: &[StreamItem]) -> Result<Vec<JsonValue>, serde_json::Error> {
    items
        .iter()
        .map(|item| match item {
            StreamItem::Json(value) => serde_json::to_value(value),
            StreamItem::Bytes(_) => Ok(JsonValue::Null),
        })
        .collect()
}

fn write_events(items: &mut [StreamItem], events: &[JsonValue]) {
    for (slot, event) in items.iter_mut().zip(events) {
        let StreamItem::Json(_) = slot else {
            continue;
        };
        *slot = StreamItem::Json(to_guardrail(event));
    }
}

mod tests;
