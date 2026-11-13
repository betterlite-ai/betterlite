use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpeechTranslationError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
}

pub async fn process_input_messages<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), SpeechTranslationError>
where
    G: GuardrailContract,
{
    match data.get("input").cloned() {
        None | Some(GuardrailValue::Null) => {
            tracing::debug!("OpenAI Text-to-Speech: No input text found in request data");
        }
        Some(GuardrailValue::Text(input)) => {
            rewrite_input(data, input, guardrail, logging_obj).await?;
        }
        Some(other) => {
            tracing::debug!(
                "OpenAI Text-to-Speech: Unexpected input type: {}. Expected string.",
                value_type_name(&other)
            );
        }
    }
    Ok(())
}

pub fn process_output_response(_response: &GuardrailValue) {
    tracing::debug!(
        "OpenAI Text-to-Speech: Output processing not applicable (output is audio data, not text)"
    );
}

async fn rewrite_input<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    input: String,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), SpeechTranslationError>
where
    G: GuardrailContract,
{
    let original_length = input.chars().count();
    let inputs = GuardrailApiInputs {
        texts: Some(vec![input.clone()]),
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
        .unwrap_or(input);
    tracing::debug!(
        "OpenAI Text-to-Speech: Applied guardrail to input text. Original length: {original_length}, New length: {}",
        rewritten.chars().count()
    );
    data.insert("input".to_owned(), GuardrailValue::Text(rewritten));
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
