use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImageTranslationError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
}

pub async fn process_input_messages<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), ImageTranslationError>
where
    G: GuardrailContract,
{
    match data.get("prompt").cloned() {
        None | Some(GuardrailValue::Null) => {
            tracing::debug!("OpenAI Image Generation: No prompt found in request data");
        }
        Some(GuardrailValue::Text(prompt)) => {
            rewrite_prompt(data, prompt, guardrail, logging_obj).await?;
        }
        Some(other) => {
            tracing::debug!(
                "OpenAI Image Generation: Unexpected prompt type: {}. Expected string.",
                value_type_name(&other)
            );
        }
    }
    Ok(())
}

pub fn process_output_response(_response: &GuardrailValue) {
    tracing::debug!(
        message = "OpenAI Image Generation: Output processing not needed for image responses"
    );
}

async fn rewrite_prompt<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    prompt: String,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), ImageTranslationError>
where
    G: GuardrailContract,
{
    let original_length = prompt.chars().count();
    let inputs = GuardrailApiInputs {
        texts: Some(vec![prompt.clone()]),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: non_empty_text(data.get("model")),
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
    tracing::debug!(
        "OpenAI Image Generation: Applied guardrail to prompt. Original length: {original_length}, New length: {}",
        rewritten.chars().count()
    );
    data.insert("prompt".to_owned(), GuardrailValue::Text(rewritten));
    Ok(())
}

fn non_empty_text(value: Option<&GuardrailValue>) -> Option<String> {
    match value {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

const fn value_type_name(value: &GuardrailValue) -> &'static str {
    match value {
        GuardrailValue::Null => "NoneType",
        GuardrailValue::Boolean(_) => "bool",
        GuardrailValue::SignedInteger(_) | GuardrailValue::UnsignedInteger(_) => "int",
        GuardrailValue::Decimal(_) => "float",
        GuardrailValue::Text(_) => "str",
        GuardrailValue::Array(_) => "list",
        GuardrailValue::Object(_) => "dict",
    }
}

mod tests;
