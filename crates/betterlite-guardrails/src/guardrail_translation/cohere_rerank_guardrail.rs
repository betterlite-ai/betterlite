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
        let scanned = scanned_fields(data);
        if scanned.is_empty() {
            tracing::debug!("Rerank: No query/instruction to process or not strings");
            return Ok(());
        }
        let inputs = GuardrailApiInputs {
            texts: Some(scanned.iter().map(|(_, text)| text.clone()).collect()),
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
        write_back(data, &scanned, guardrailed.texts.as_deref());
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

// an empty string is still scanned, so the check is on the type and not the value
fn scanned_fields(data: &BTreeMap<String, GuardrailValue>) -> Vec<(&'static str, String)> {
    let mut scanned = Vec::new();
    for name in SCANNED_FIELDS {
        if let Some(GuardrailValue::Text(text)) = data.get(name) {
            scanned.push((name, text.clone()));
        }
    }
    scanned
}

// the hook's positions line up with the scanned fields, and a missing position keeps the original
fn write_back(
    data: &mut BTreeMap<String, GuardrailValue>,
    scanned: &[(&'static str, String)],
    texts: Option<&[String]>,
) {
    let Some(returned) = texts else {
        return;
    };
    for (index, (name, original)) in scanned.iter().enumerate() {
        let Some(rewritten) = returned.get(index) else {
            continue;
        };
        data.insert((*name).to_owned(), GuardrailValue::Text(rewritten.clone()));
        tracing::debug!(
            "Rerank: Applied guardrail to {name}. Original length: {}, New length: {}",
            original.chars().count(),
            rewritten.chars().count()
        );
    }
}

fn non_empty_model(value: Option<&GuardrailValue>) -> Option<String> {
    match value {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.clone()),
        Some(_) | None => None,
    }
}
