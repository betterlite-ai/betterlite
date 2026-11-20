#![expect(
    clippy::disallowed_types,
    reason = "Anthropic response bodies and content blocks are provider JSON that betterlite does not model"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

pub mod error;
mod tests;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use serde_json::{Map, Value};

use crate::guardrails::anthropic_translation::input_scan::serde_convert;
use crate::guardrails::anthropic_translation::input_scan::tool_calls::output_tool_use_to_openai;
use crate::guardrails::response_context::add_response_context;

use error::ProcessOutputError;

const CONTENT: &str = "content";
const MODEL: &str = "model";
const TEXT: &str = "text";
const TOOL_USE: &str = "tool_use";
const TYPE: &str = "type";

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExtractedOutput {
    pub(crate) texts: Vec<String>,
    pub(crate) mappings: Vec<usize>,
    pub(crate) tool_calls: Vec<Value>,
}

pub(crate) fn extract_from_content_blocks(
    content: &[Value],
) -> Result<ExtractedOutput, ProcessOutputError> {
    let mut extracted = ExtractedOutput {
        texts: Vec::new(),
        mappings: Vec::new(),
        tool_calls: Vec::new(),
    };
    for (content_idx, block) in content.iter().enumerate() {
        let Some(fields) = block.as_object() else {
            continue;
        };
        match fields.get(TYPE).and_then(Value::as_str) {
            Some(TEXT) => {
                let found_text = fields
                    .get(TEXT)
                    .and_then(Value::as_str)
                    .filter(|candidate| !candidate.is_empty());
                if let Some(text) = found_text {
                    extracted.texts.push(text.to_owned());
                    extracted.mappings.push(content_idx);
                }
            }
            Some(TOOL_USE) => {
                let tool_call = output_tool_use_to_openai(fields, content_idx)
                    .ok_or(ProcessOutputError::MalformedToolUse { content_idx })?;
                extracted.tool_calls.push(tool_call);
            }
            Some(_) | None => {}
        }
    }
    Ok(extracted)
}

// LiteLLM pairs each returned text with the content position of the text it replaced
pub fn apply_responses_to_output(
    content: &mut [Value],
    responses: &[String],
    mappings: &[usize],
) -> Result<(), ProcessOutputError> {
    if responses.len() > mappings.len() {
        return Err(ProcessOutputError::TooManyTexts {
            sent: mappings.len(),
            returned: responses.len(),
        });
    }
    for (response, content_idx) in responses.iter().zip(mappings) {
        let Some(block) = content.get_mut(*content_idx) else {
            continue;
        };
        let Some(fields) = block.as_object_mut() else {
            continue;
        };
        if fields.get(TYPE).and_then(Value::as_str) == Some(TEXT) {
            fields.insert(TEXT.to_owned(), Value::String(response.clone()));
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
) -> Result<(), ProcessOutputError>
where
    G: GuardrailContract,
{
    let body: Map<String, Value> = serde_convert(response, "response")?;
    let content = response_content(&body);
    if content.is_empty() {
        return Ok(());
    }
    let extracted = extract_from_content_blocks(&content)?;
    if extracted.texts.is_empty() && extracted.tool_calls.is_empty() {
        return Ok(());
    }
    let mut fresh = BTreeMap::new();
    let context_data = match request_data {
        Some(data) => data,
        None => &mut fresh,
    };
    add_response_context(context_data, response, user_api_key_dict);
    let inputs = output_inputs(&body, &extracted)?;
    let answer = run_output_guardrail(guardrail, inputs, context_data, logging_obj).await?;
    let returned = answer.texts.unwrap_or_default();
    let mut written_content = content;
    apply_responses_to_output(&mut written_content, &returned, &extracted.mappings)?;
    let mut written_body = body;
    written_body.insert(CONTENT.to_owned(), Value::Array(written_content));
    tracing::debug!("Anthropic Messages: Processed output response: {written_body:?}");
    *response = serde_convert(&written_body, "response")?;
    Ok(())
}

fn response_content(body: &Map<String, Value>) -> Vec<Value> {
    body.get(CONTENT)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn output_inputs(
    body: &Map<String, Value>,
    extracted: &ExtractedOutput,
) -> Result<GuardrailApiInputs, ProcessOutputError> {
    let mut inputs = GuardrailApiInputs {
        texts: Some(extracted.texts.clone()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    if !extracted.tool_calls.is_empty() {
        let mut tool_calls = Vec::with_capacity(extracted.tool_calls.len());
        for tool_call in &extracted.tool_calls {
            tool_calls.push(serde_convert::<_, GuardrailValue>(tool_call, "tool call")?);
        }
        inputs.tool_calls = Some(tool_calls);
    }
    if let Some(Value::String(model)) = body.get(MODEL)
        && !model.is_empty()
    {
        inputs.model = Some(model.clone());
    }
    Ok(inputs)
}

async fn run_output_guardrail<G>(
    guardrail: &G,
    inputs: GuardrailApiInputs,
    context_data: &mut BTreeMap<String, GuardrailValue>,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<GuardrailApiInputs, ProcessOutputError>
where
    G: GuardrailContract,
{
    guardrail
        .apply_guardrail(
            inputs,
            context_data,
            GuardrailInputType::Response,
            logging_obj,
        )
        .await
        .map_err(|source| ProcessOutputError::Guardrail {
            source: Box::new(source),
        })
}
