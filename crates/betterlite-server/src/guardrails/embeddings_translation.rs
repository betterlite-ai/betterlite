use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbeddingsTranslationError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error("embedding input item {index} is {kind}, not text")]
    NonTextItem { index: usize, kind: &'static str },
}

pub async fn process_input_messages<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), EmbeddingsTranslationError>
where
    G: GuardrailContract,
{
    match data.get("input").cloned() {
        None | Some(GuardrailValue::Null) => {
            tracing::debug!("OpenAI Embeddings: No input found in request data");
        }
        Some(GuardrailValue::Text(input)) => {
            rewrite_string_input(data, input, guardrail, logging_obj).await?;
        }
        Some(GuardrailValue::Array(items)) => {
            rewrite_list_input(data, items, guardrail, logging_obj).await?;
        }
        Some(other) => {
            tracing::warn!(
                "OpenAI Embeddings: Unexpected input type: {}. Expected string or list.",
                value_type_name(&other)
            );
        }
    }
    Ok(())
}

pub fn process_output_response(_response: &GuardrailValue) {
    tracing::debug!(
        "OpenAI Embeddings: Output response processing skipped - embeddings contain vectors, not text"
    );
}

async fn rewrite_string_input<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    input: String,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), EmbeddingsTranslationError>
where
    G: GuardrailContract,
{
    let original_length = input.chars().count();
    let inputs = GuardrailApiInputs {
        texts: Some(vec![input]),
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
    let Some(rewritten) = guardrailed.texts.as_deref().and_then(<[String]>::first) else {
        return Ok(());
    };
    tracing::debug!(
        "OpenAI Embeddings: Applied guardrail to string input. Original length: {original_length}, New length: {}",
        rewritten.chars().count()
    );
    data.insert("input".to_owned(), GuardrailValue::Text(rewritten.clone()));
    Ok(())
}

async fn rewrite_list_input<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    items: Vec<GuardrailValue>,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), EmbeddingsTranslationError>
where
    G: GuardrailContract,
{
    let Some(texts) = collected_list_texts(&items)? else {
        return Ok(());
    };
    let inputs = GuardrailApiInputs {
        texts: Some(texts),
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
    let Some(returned) = guardrailed.texts else {
        return Ok(());
    };
    if returned.is_empty() {
        return Ok(());
    }
    tracing::debug!(
        "OpenAI Embeddings: Applied guardrail to {} inputs",
        returned.len()
    );
    let rewritten = returned.into_iter().map(GuardrailValue::Text).collect();
    data.insert("input".to_owned(), GuardrailValue::Array(rewritten));
    Ok(())
}

fn collected_list_texts(
    items: &[GuardrailValue],
) -> Result<Option<Vec<String>>, EmbeddingsTranslationError> {
    let Some(first) = items.first() else {
        return Ok(None);
    };
    match first {
        GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Array(_) => {
            tracing::debug!("OpenAI Embeddings: Input is token IDs, skipping guardrail processing");
            return Ok(None);
        }
        GuardrailValue::Text(_) => {}
        other @ (GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Object(_)) => {
            tracing::warn!(
                "OpenAI Embeddings: Unexpected input list item type: {}",
                value_type_name(other)
            );
            return Ok(None);
        }
    }
    let mut texts = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let GuardrailValue::Text(text) = item else {
            return Err(EmbeddingsTranslationError::NonTextItem {
                index,
                kind: value_type_name(item),
            });
        };
        texts.push(text.clone());
    }
    Ok(Some(texts))
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
