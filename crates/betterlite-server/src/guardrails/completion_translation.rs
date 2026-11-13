use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use thiserror::Error;

use super::response_context::add_response_context;

#[derive(Debug, Error)]
pub enum CompletionTranslationError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
}

pub async fn process_input_messages<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), CompletionTranslationError>
where
    G: GuardrailContract,
{
    match data.get("prompt").cloned() {
        None | Some(GuardrailValue::Null) => {
            tracing::debug!("OpenAI Text Completion: No prompt found in request data");
        }
        Some(GuardrailValue::Text(prompt)) => {
            rewrite_string_prompt(data, prompt, guardrail, logging_obj).await?;
        }
        Some(GuardrailValue::Array(prompts)) => {
            rewrite_prompt_array(data, prompts, guardrail, logging_obj).await?;
        }
        Some(other) => {
            tracing::warn!(
                "OpenAI Text Completion: Unexpected prompt type: {}. Expected string or list.",
                value_type_name(&other)
            );
        }
    }
    Ok(())
}

pub async fn process_output_response<G>(
    response: &mut GuardrailValue,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
    user_api_key_dict: Option<&BTreeMap<String, GuardrailValue>>,
    request_data: Option<&mut BTreeMap<String, GuardrailValue>>,
) -> Result<(), CompletionTranslationError>
where
    G: GuardrailContract,
{
    let Some(choices) = choice_texts(response) else {
        tracing::debug!("OpenAI Text Completion: No choices in response to process");
        return Ok(());
    };
    if choices.is_empty() {
        return Ok(());
    }
    let mut fresh_request_data = BTreeMap::new();
    let prepared_request_data = request_data.unwrap_or(&mut fresh_request_data);
    add_response_context(prepared_request_data, response, user_api_key_dict);
    let inputs = GuardrailApiInputs {
        texts: Some(choices.iter().map(|(_index, text)| text.clone()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: non_empty_text(object_field(response, "model")),
        stream_holdback_chars: None,
    };
    let guardrailed = guardrail
        .apply_guardrail(
            inputs,
            prepared_request_data,
            GuardrailInputType::Response,
            logging_obj,
        )
        .await?;
    if let Some(returned) = &guardrailed.texts {
        rewrite_choices(response, &choices, returned);
    }
    Ok(())
}

async fn rewrite_string_prompt<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    prompt: String,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), CompletionTranslationError>
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
        "OpenAI Text Completion: Applied guardrail to string prompt. Original length: {original_length}, New length: {}",
        rewritten.chars().count()
    );
    data.insert("prompt".to_owned(), GuardrailValue::Text(rewritten));
    Ok(())
}

async fn rewrite_prompt_array<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    prompts: Vec<GuardrailValue>,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), CompletionTranslationError>
where
    G: GuardrailContract,
{
    let collected = collected_texts(&prompts);
    if collected.is_empty() {
        return Ok(());
    }
    let inputs = GuardrailApiInputs {
        texts: Some(
            collected
                .iter()
                .map(|(_index, text)| text.clone())
                .collect(),
        ),
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
    rewrite_prompts(data, &collected, &returned);
    Ok(())
}

fn collected_texts(values: &[GuardrailValue]) -> Vec<(usize, String)> {
    let mut collected = Vec::new();
    for (index, value) in values.iter().enumerate() {
        if let GuardrailValue::Text(text) = value {
            collected.push((index, text.clone()));
        }
    }
    collected
}

fn choice_texts(response: &GuardrailValue) -> Option<Vec<(usize, String)>> {
    let GuardrailValue::Object(fields) = response else {
        return None;
    };
    let Some(GuardrailValue::Array(choices)) = fields.get("choices") else {
        return None;
    };
    if choices.is_empty() {
        return None;
    }
    Some(collected_texts(choices))
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

fn rewrite_prompts(
    data: &mut BTreeMap<String, GuardrailValue>,
    collected: &[(usize, String)],
    returned: &[String],
) {
    for ((prompt_index, original), rewritten) in collected.iter().zip(returned.iter()) {
        write_prompt_at(data, *prompt_index, rewritten);
        tracing::debug!(
            "OpenAI Text Completion: Applied guardrail to prompt[{prompt_index}]. Original length: {}, New length: {}",
            original.chars().count(),
            rewritten.chars().count()
        );
    }
}

fn rewrite_choices(
    response: &mut GuardrailValue,
    choices: &[(usize, String)],
    returned: &[String],
) {
    for ((choice_index, original), rewritten) in choices.iter().zip(returned.iter()) {
        write_choice_text(response, *choice_index, rewritten);
        tracing::debug!(
            "OpenAI Text Completion: Applied guardrail to choice[{choice_index}] text. Original length: {}, New length: {}",
            original.chars().count(),
            rewritten.chars().count()
        );
    }
}

fn write_prompt_at(data: &mut BTreeMap<String, GuardrailValue>, index: usize, text: &str) {
    let Some(GuardrailValue::Array(prompts)) = data.get_mut("prompt") else {
        return;
    };
    let Some(slot) = prompts.get_mut(index) else {
        return;
    };
    *slot = GuardrailValue::Text(text.to_owned());
}

fn write_choice_text(response: &mut GuardrailValue, choice_index: usize, text: &str) {
    let GuardrailValue::Object(fields) = response else {
        return;
    };
    let Some(GuardrailValue::Array(choices)) = fields.get_mut("choices") else {
        return;
    };
    let Some(GuardrailValue::Object(choice)) = choices.get_mut(choice_index) else {
        return;
    };
    choice.insert("text".to_owned(), GuardrailValue::Text(text.to_owned()));
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
