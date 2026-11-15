use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, StreamingScanKey, TranslationCall,
};

use super::{A2aTranslationError, input, output, stream};

pub const A2A_CALL_TYPES: [CallType; 2] = [CallType::SendMessage, CallType::AsendMessage];

pub struct A2ATranslation<G> {
    pub guardrail: PhantomData<fn() -> G>,
}

impl<G: GuardrailContract> BaseTranslation for A2ATranslation<G> {
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = A2aTranslationError;

    const DELIVERS_ENDED_STREAM_REWRITES: bool = false;
    const ASSEMBLES_STREAMED_RESPONSE: bool = false;

    fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        input::process_input_messages(data, guardrail, logging_obj)
    }

    async fn process_output_response(
        &self,
        mut response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        output::process_output_response(
            &mut response,
            guardrail,
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
        stream::process_streaming_response(
            responses_so_far,
            guardrail,
            call.logging_obj,
            call.user_api_key_dict,
            call.request_data,
        )
    }

    fn get_streaming_scan_key(
        &self,
        responses_so_far: &[GuardrailValue],
    ) -> Option<StreamingScanKey> {
        Some(stream::get_streaming_scan_key(responses_so_far))
    }
}

mod tests;
