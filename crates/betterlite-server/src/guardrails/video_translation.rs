use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VideoTranslationError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
}

pub async fn process_input_messages<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), VideoTranslationError>
where
    G: GuardrailContract,
{
    let Some(GuardrailValue::Text(prompt)) = data.get("prompt").cloned() else {
        return Ok(());
    };
    let inputs = GuardrailApiInputs {
        texts: Some(vec![prompt.clone()]),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: text_value(data.get("model")),
        stream_holdback_chars: None,
    };
    let guardrailed = guardrail
        .apply_guardrail(inputs, data, GuardrailInputType::Request, logging_obj)
        .await?;
    let rewritten = guardrailed
        .texts
        .as_deref()
        .and_then(<[String]>::first)
        .cloned()
        .unwrap_or(prompt);
    data.insert("prompt".to_owned(), GuardrailValue::Text(rewritten));
    Ok(())
}

pub const fn process_output_response(_response: &GuardrailValue) {}

fn text_value(value: Option<&GuardrailValue>) -> Option<String> {
    match value {
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

mod tests;
