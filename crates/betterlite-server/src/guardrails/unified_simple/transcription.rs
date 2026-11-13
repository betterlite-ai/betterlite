use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use crate::guardrails::transcription_translation::{self, TranscriptionTranslationError};

// BaseTranslation fixes one Guardrail type per impl, so the adapter carries it as its only type parameter
pub struct TranscriptionTranslation<G: GuardrailContract>(PhantomData<G>);

impl<G: GuardrailContract> Default for TranscriptionTranslation<G> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<G: GuardrailContract> BaseTranslation for TranscriptionTranslation<G> {
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = TranscriptionTranslationError;

    fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        _guardrail: &Self::Guardrail,
        _logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        transcription_translation::process_input_messages(data);
        ready(Ok(()))
    }

    async fn process_output_response(
        &self,
        mut response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        transcription_translation::process_output_response(
            &mut response,
            guardrail,
            call.logging_obj,
            call.user_api_key_dict,
            call.request_data,
        )
        .await?;
        Ok(response)
    }
}

mod tests;
