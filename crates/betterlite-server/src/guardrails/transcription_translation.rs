use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use thiserror::Error;

use super::response_context::add_response_context;

#[derive(Debug, Error)]
pub enum TranscriptionTranslationError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
}

pub fn process_input_messages(_data: &BTreeMap<String, GuardrailValue>) {
    tracing::debug!(
        "OpenAI Audio Transcription: Input processing not applicable (input is audio file, not text)"
    );
}

pub async fn process_output_response<G>(
    response: &mut GuardrailValue,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
    user_api_key_dict: Option<&BTreeMap<String, GuardrailValue>>,
    request_data: Option<&mut BTreeMap<String, GuardrailValue>>,
) -> Result<(), TranscriptionTranslationError>
where
    G: GuardrailContract,
{
    let Some(text) = transcribed_text(response) else {
        return Ok(());
    };
    let original_length = text.chars().count();
    let mut fresh_request_data = BTreeMap::new();
    let prepared_request_data = request_data.unwrap_or(&mut fresh_request_data);
    add_response_context(prepared_request_data, response, user_api_key_dict);
    let guardrailed = guardrail
        .apply_guardrail(
            transcript_inputs(&text, response),
            prepared_request_data,
            GuardrailInputType::Response,
            logging_obj,
        )
        .await?;
    let rewritten = guardrailed
        .texts
        .as_deref()
        .and_then(<[String]>::first)
        .cloned()
        .unwrap_or(text);
    tracing::debug!(
        "OpenAI Audio Transcription: Applied guardrail to transcribed text. Original length: {original_length}, New length: {}",
        rewritten.chars().count()
    );
    if let GuardrailValue::Object(fields) = response {
        fields.insert("text".to_owned(), GuardrailValue::Text(rewritten));
    }
    Ok(())
}

fn transcribed_text(response: &GuardrailValue) -> Option<String> {
    let GuardrailValue::Object(fields) = response else {
        tracing::debug!("OpenAI Audio Transcription: No text in response to process");
        return None;
    };
    match fields.get("text") {
        None | Some(GuardrailValue::Null) => {
            tracing::debug!("OpenAI Audio Transcription: No text in response to process");
            None
        }
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        Some(other) => {
            tracing::debug!(
                "OpenAI Audio Transcription: Unexpected text type: {}. Expected string.",
                value_type_name(other)
            );
            None
        }
    }
}

fn transcript_inputs(text: &str, response: &GuardrailValue) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(vec![text.to_owned()]),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: non_empty_text(object_field(response, "model")),
        stream_holdback_chars: None,
    }
}

fn object_field<'value>(
    value: &'value GuardrailValue,
    key: &str,
) -> Option<&'value GuardrailValue> {
    let GuardrailValue::Object(fields) = value else {
        return None;
    };
    fields.get(key)
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
