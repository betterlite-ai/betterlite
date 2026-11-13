use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use crate::guardrails::image_translation::{self, ImageTranslationError};

// BaseTranslation fixes one Guardrail type per impl, so the adapter carries it as its only type parameter
pub struct ImageTranslation<G: GuardrailContract>(PhantomData<G>);

impl<G: GuardrailContract> Default for ImageTranslation<G> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<G: GuardrailContract> BaseTranslation for ImageTranslation<G> {
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = ImageTranslationError;

    fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        image_translation::process_input_messages(data, guardrail, logging_obj)
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        _guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        image_translation::process_output_response(&response);
        ready(Ok(response))
    }
}

mod tests;
