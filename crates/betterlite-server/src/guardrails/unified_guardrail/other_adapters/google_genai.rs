use std::collections::BTreeMap;
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::stream_item_value;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, TranslationCall, transform_user_key_metadata,
};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::google_genai_guardrail::CALL_TYPES;
use betterlite_vertex::google_genai_guardrail::callbacks::{
    ApplyFuture, GenaiGuardrail, GuardedOutput, GuardrailCall, GuardrailInputs as GenaiInputs,
    InputType, process_output, process_request,
};

use super::converted;

pub const GOOGLE_GENAI_CALL_TYPES: [CallType; 4] = CALL_TYPES;

pub struct GoogleGenAiTranslation<G>(pub PhantomData<fn() -> G>);

// B1391's handler speaks its own guardrail trait over prompt JSON, so each call crosses through this bridge
struct GenaiBridge<'call, G> {
    guardrail: &'call G,
    logging_obj: Option<&'call mut GuardrailLogTarget>,
}

impl<G: GuardrailContract> GenaiGuardrail for GenaiBridge<'_, G> {
    type Failure = GuardrailFailure;

    fn apply_guardrail<'call>(
        &'call mut self,
        inputs: GenaiInputs,
        call: GuardrailCall<'call>,
    ) -> ApplyFuture<'call, Self::Failure> {
        Box::pin(async move {
            let mut request_data: BTreeMap<String, GuardrailValue> = converted(call.request_data)?;
            let input_type = match call.input_type {
                InputType::Request => GuardrailInputType::Request,
                InputType::Response => GuardrailInputType::Response,
            };
            let sent = GuardrailApiInputs {
                texts: Some(inputs.texts),
                images: None,
                tools: None,
                tool_calls: None,
                structured_messages: None,
                model: inputs.model,
                stream_holdback_chars: None,
            };
            let returned = self
                .guardrail
                .apply_guardrail(
                    sent,
                    &mut request_data,
                    input_type,
                    self.logging_obj.as_deref_mut(),
                )
                .await?;
            Ok(returned.texts)
        })
    }
}

// the response and the stream scan read the request and the key metadata the same way
async fn scan_output<G>(
    output: GuardedOutput<'_>,
    guardrail: &G,
    call: TranslationCall<'_, GuardrailLogTarget>,
) -> Result<(), GuardrailFailure>
where
    G: GuardrailContract,
{
    let request: Option<Object> = match call.request_data.as_deref() {
        Some(data) => Some(converted(data)?),
        None => None,
    };
    let metadata = converted(&transform_user_key_metadata(call.user_api_key_dict))?;
    let mut bridge = GenaiBridge {
        guardrail,
        logging_obj: call.logging_obj,
    };
    process_output(output, request.as_ref(), &metadata, &mut bridge).await
}

impl<G: GuardrailContract> BaseTranslation for GoogleGenAiTranslation<G> {
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = GuardrailFailure;

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let mut request: Object = converted(&*data)?;
        let mut bridge = GenaiBridge {
            guardrail,
            logging_obj,
        };
        process_request(&mut request, &mut bridge).await?;
        *data = converted(&request)?;
        Ok(())
    }

    async fn process_output_response(
        &self,
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        let GuardrailValue::Object(fields) = response else {
            return Ok(response);
        };
        let mut body: Object = converted(&fields)?;
        scan_output(GuardedOutput::Response(&mut body), guardrail, call).await?;
        converted(&body)
    }

    async fn process_streaming_response(
        &self,
        responses_so_far: &mut Vec<StreamItem>,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let frames = responses_so_far
            .iter()
            .map(|item| converted(&stream_item_value(item)))
            .collect::<Result<Vec<Json>, _>>()?;
        scan_output(GuardedOutput::Frames(&frames), guardrail, call).await
    }
}

mod tests;
