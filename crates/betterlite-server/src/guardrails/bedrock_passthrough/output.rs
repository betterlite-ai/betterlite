use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use thiserror::Error;

use super::routing::{Routed, is_converse_endpoint};
use super::text_paths::steps::TextPath;
use super::text_paths::{collect_output_texts, write_back_texts};

mod tests;

#[derive(Debug, Error)]
pub enum OutputTranslationError {
    #[error("the guardrail failed: {0}")]
    Guardrail(#[from] GuardrailFailure),
}

pub async fn process_output_response<G>(
    response: &mut GuardrailValue,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
    user_metadata: BTreeMap<String, GuardrailValue>,
    request_data: Option<&BTreeMap<String, GuardrailValue>>,
) -> Result<Routed, OutputTranslationError>
where
    G: GuardrailContract,
{
    let endpoint = request_data
        .and_then(|data| text_field(data, "endpoint"))
        .unwrap_or_default();
    if !endpoint.is_empty() && !is_converse_endpoint(endpoint) {
        return Ok(Routed::Generic);
    }
    let Some(collected) = output_texts(response) else {
        return Ok(Routed::Handled);
    };
    let (holders, texts): (Vec<TextPath>, Vec<String>) = collected.into_iter().unzip();
    let mut guarded_request_data = request_data_with_metadata(request_data, user_metadata);
    let inputs = GuardrailApiInputs {
        texts: Some(texts),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: text_field(&guarded_request_data, "model")
            .filter(|model| !model.is_empty())
            .map(str::to_owned),
        stream_holdback_chars: None,
    };
    let guardrailed = guardrail
        .apply_guardrail(
            inputs,
            &mut guarded_request_data,
            GuardrailInputType::Response,
            logging_obj,
        )
        .await?;
    if let Some(returned) = guardrailed.texts
        && !returned.is_empty()
        && let Some(content) = content_blocks_mut(response)
    {
        write_back_texts(content, &holders, &returned);
    }
    Ok(Routed::Handled)
}

fn output_texts(response: &mut GuardrailValue) -> Option<Vec<(TextPath, String)>> {
    let Some(GuardrailValue::Array(blocks)) = content_blocks_mut(response) else {
        return None;
    };
    let collected = collect_output_texts(blocks);
    if collected.is_empty() {
        return None;
    }
    Some(collected)
}

fn text_field<'data>(
    request_data: &'data BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Option<&'data str> {
    match request_data.get(key) {
        Some(GuardrailValue::Text(text)) => Some(text),
        _ => None,
    }
}

// LiteLLM builds this copy inside the handler, so the caller's request data never gains the key.
fn request_data_with_metadata(
    request_data: Option<&BTreeMap<String, GuardrailValue>>,
    user_metadata: BTreeMap<String, GuardrailValue>,
) -> BTreeMap<String, GuardrailValue> {
    let mut copied = request_data.cloned().unwrap_or_default();
    if !copied.contains_key("litellm_metadata") && !user_metadata.is_empty() {
        copied.insert(
            "litellm_metadata".to_owned(),
            GuardrailValue::Object(user_metadata),
        );
    }
    copied
}

fn content_blocks_mut(response: &mut GuardrailValue) -> Option<&mut GuardrailValue> {
    let GuardrailValue::Object(fields) = response else {
        return None;
    };
    let Some(GuardrailValue::Object(output)) = fields.get_mut("output") else {
        return None;
    };
    let Some(GuardrailValue::Object(message)) = output.get_mut("message") else {
        return None;
    };
    match message.get_mut("content") {
        Some(content @ GuardrailValue::Array(_)) => Some(content),
        _ => None,
    }
}
