use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use super::any_guardrail::AnyGuardrail;
use crate::guardrails::speech_translation::{self, SpeechTranslationError};

pub struct SpeechTranslation;

impl BaseTranslation for SpeechTranslation {
    type Guardrail = AnyGuardrail;
    type LogTarget = GuardrailLogTarget;
    type Failure = SpeechTranslationError;

    fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        speech_translation::process_input_messages(data, guardrail, logging_obj)
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        _guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        speech_translation::process_output_response(&response);
        ready(Ok(response))
    }
}

mod tests;
