use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use crate::guardrails::completion_translation::{self, CompletionTranslationError};

// BaseTranslation fixes one Guardrail type per impl, so the adapter carries it as its only type parameter
pub struct CompletionTranslation<G: GuardrailContract>(PhantomData<G>);

impl<G: GuardrailContract> Default for CompletionTranslation<G> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<G: GuardrailContract> BaseTranslation for CompletionTranslation<G> {
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = CompletionTranslationError;

    fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        completion_translation::process_input_messages(data, guardrail, logging_obj)
    }

    async fn process_output_response(
        &self,
        mut response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        completion_translation::process_output_response(
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
