use std::collections::BTreeMap;
use std::future::Future;
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
use betterlite_guardrails::guardrail_message_scope::scoped_indices::ScopeFlags;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, StreamingScanKey, TranslationCall,
};
use thiserror::Error;

use super::stamps::StampSource;
use super::undeliverable;
use super::{GuardrailToApply, failure_message, modify_response, to_json};
use crate::guardrails::responses_translation::block_events;
use crate::guardrails::responses_translation::block_events::guardrail_json::{
    blocked_original_response, to_guardrail,
};
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
}

impl ResponsesTranslationError {
    fn guardrail(&self) -> Option<&GuardrailFailure> {
        match self {
            Self::Request(RequestRewriteError::Guardrail { source }) => Some(source.as_ref()),
            Self::Output(OutputRewriteError::Guardrail { source }) => Some(source.as_ref()),
            Self::Stream(StreamOutputError::Guardrail(failure)) => Some(failure),
            Self::Request(_) | Self::Output(_) | Self::Stream(_) | Self::Undeliverable(_) => None,
        }
    }
}

pub struct ResponsesTranslation<G, S> {
    stamps: S,
    limits: ApiLimits,
    cache: Arc<DualLocalCache>,
    _guardrail: PhantomData<fn() -> G>,
}

impl<G, S> ResponsesTranslation<G, S> {
    pub fn new(stamps: S, limits: ApiLimits, cache: Arc<DualLocalCache>) -> Self {
        Self {
            stamps,
            limits,
            cache,
            _guardrail: PhantomData,
        }
    }
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

    fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
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
        let applied = guardrail.guardrail.as_ref();
        async move {
            request_rewrite::process_input_messages(data, applied, &context, logging_obj).await?;
            Ok(())
        }
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

    fn process_streaming_response(
        &self,
        responses_so_far: &mut Vec<GuardrailValue>,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        let mut events = to_events(responses_so_far);
        let name = guardrail.guardrail_name.as_deref();
        let applied = guardrail.guardrail.as_ref();
        let logged = call.logging_obj;
        let user_api_key_dict = call.user_api_key_dict;
        let request_data = call.request_data;
        let deliver = call.deliver_ended_stream_rewrites;
        async move {
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
    }

    fn get_streaming_scan_key(
        &self,
        responses_so_far: &[GuardrailValue],
    ) -> Option<StreamingScanKey> {
        stream_scan::streaming_scan_key(&to_events(responses_so_far))
    }

    fn build_block_sse_chunks(
        &self,
        failure: &Self::Failure,
        stream_started: bool,
        responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<Vec<u8>>> {
        let details = modify_response(failure.guardrail()?)?;
        let original = details.original_response.as_ref().map(to_json);
        let events = responses_so_far.map(to_events);
        block_events::build_block_sse_chunks(
            &details.message,
            &details.model,
            original.as_ref(),
            stream_started,
            events.as_deref(),
            &self.stamps.stamp(),
        )
        .ok()
    }

    fn build_stream_error_items(
        &self,
        failure: &Self::Failure,
        responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<GuardrailValue>> {
        let (status, message) = failure_message(failure.guardrail()?)?;
        let events = responses_so_far.map(to_events);
        block_events::build_stream_error_items(status, &message, events.as_deref())
            .into_iter()
            .map(|event| {
                serde_json::to_value(event)
                    .ok()
                    .map(|value| to_guardrail(&value))
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

fn to_events(items: &[GuardrailValue]) -> Vec<JsonValue> {
    items.iter().map(to_json).collect()
}

fn write_events(items: &mut [GuardrailValue], events: &[JsonValue]) {
    for (slot, event) in items.iter_mut().zip(events) {
        *slot = to_guardrail(event);
    }
}

mod tests;
