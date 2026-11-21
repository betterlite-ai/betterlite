use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::cohere_rerank_guardrail::{
    CohereRerankGuardrailError, CohereRerankHandler,
};
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub const RERANK_CALL_TYPES: [CallType; 2] = [CallType::Rerank, CallType::Arerank];

pub struct RerankTranslation<G>(pub PhantomData<fn() -> G>);

fn converted<Source, Target>(value: &Source) -> Result<Target, GuardrailFailure>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|error| {
            GuardrailFailure::Other(format!("guardrail data could not be converted: {error}"))
        })
}

impl<G: GuardrailContract> BaseTranslation for RerankTranslation<G> {
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = GuardrailFailure;

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        CohereRerankHandler
            .process_input_messages(data, guardrail, logging_obj)
            .await
            .map_err(|CohereRerankGuardrailError::Guardrail(failure)| failure)
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        ready(Ok(
            CohereRerankHandler.process_output_response(response, guardrail)
        ))
    }
}

pub mod google_genai;
pub mod mcp;
mod tests;
