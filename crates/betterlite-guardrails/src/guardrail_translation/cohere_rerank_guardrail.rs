use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use thiserror::Error;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

mod tests;

// LiteLLM's CohereRerankHandler scans both request strings and leaves documents to the corpus
const SCANNED_FIELDS: [&str; 2] = ["query", "instruction"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RerankCallType {
    Rerank,
    Arerank,
}

// B037 converts these two names to its global call type when it fills the registry
pub const GUARDRAIL_TRANSLATION_MAPPINGS: [(RerankCallType, CohereRerankHandler); 2] = [
    (RerankCallType::Rerank, CohereRerankHandler),
    (RerankCallType::Arerank, CohereRerankHandler),
];

#[derive(Debug, Error)]
pub enum CohereRerankGuardrailError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CohereRerankHandler;

impl CohereRerankHandler {
    pub async fn process_input_messages<G>(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &G,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<(), CohereRerankGuardrailError>
    where
        G: GuardrailContract,
    {
        // an empty string is still scanned, so the check is on the type and not the value
        let (names, texts): (Vec<_>, Vec<_>) = SCANNED_FIELDS
            .into_iter()
            .filter_map(|name| match data.get(name) {
                Some(GuardrailValue::Text(text)) => Some((name, text.clone())),
                Some(_) | None => None,
            })
            .unzip();
        if names.is_empty() {
            tracing::debug!("Rerank: No query/instruction to process or not strings");
            return Ok(());
        }
        let inputs = GuardrailApiInputs {
            texts: Some(texts),
            images: None,
            tools: None,
            tool_calls: None,
            structured_messages: None,
            model: non_empty_model(data.get("model")),
            stream_holdback_chars: None,
        };
        let guardrailed = guardrail
            .apply_guardrail(inputs, data, GuardrailInputType::Request, logging_obj)
            .await?;
        // a position the hook did not return keeps its original field
        let returned = guardrailed.texts.unwrap_or_default();
        for (name, text) in names.into_iter().zip(returned) {
            data.insert(name.to_owned(), GuardrailValue::Text(text));
        }
        Ok(())
    }

    // LiteLLM's process_output_response logs and returns the response, because a rerank reply has no text
    pub fn process_output_response<G>(
        &self,
        response: GuardrailValue,
        _guardrail: &G,
    ) -> GuardrailValue
    where
        G: GuardrailContract,
    {
        tracing::debug!(
            "Rerank: Output processing not applicable (output contains relevance scores, not text)"
        );
        response
    }
}

fn non_empty_model(value: Option<&GuardrailValue>) -> Option<String> {
    match value {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.clone()),
        Some(_) | None => None,
    }
}
