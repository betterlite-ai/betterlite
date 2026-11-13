use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use thiserror::Error;

use super::routing::{Routed, is_converse_endpoint};
use super::text_paths::steps::TextPath;
use super::text_paths::{SkipFlags, collect_request_texts, write_back_texts};

mod tests;

#[derive(Debug, Error)]
pub enum RequestTranslationError {
    #[error("the guardrail failed: {0}")]
    Guardrail(#[from] GuardrailFailure),
}

pub async fn process_input_messages<G>(
    data: &mut BTreeMap<String, GuardrailValue>,
    guardrail: &G,
    skip: SkipFlags,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<Routed, RequestTranslationError>
where
    G: GuardrailContract,
{
    let endpoint = match data.get("endpoint") {
        Some(GuardrailValue::Text(endpoint)) => endpoint.as_str(),
        _ => "",
    };
    if !is_converse_endpoint(endpoint) {
        return Ok(Routed::Generic);
    }
    let Some(collected) = request_texts(data, skip) else {
        return Ok(Routed::Handled);
    };
    let (holders, texts): (Vec<TextPath>, Vec<String>) = collected.into_iter().unzip();
    let model = match data.get("model") {
        Some(GuardrailValue::Text(model)) if !model.is_empty() => Some(model.clone()),
        _ => None,
    };
    let inputs = GuardrailApiInputs {
        texts: Some(texts),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model,
        stream_holdback_chars: None,
    };
    let guardrailed = guardrail
        .apply_guardrail(inputs, data, GuardrailInputType::Request, logging_obj)
        .await?;
    if let Some(returned) = guardrailed.texts
        && !returned.is_empty()
        && let Some(body) = data.get_mut("data")
    {
        write_back_texts(body, &holders, &returned);
    }
    Ok(Routed::Handled)
}

fn request_texts(
    data: &BTreeMap<String, GuardrailValue>,
    skip: SkipFlags,
) -> Option<Vec<(TextPath, String)>> {
    let body = data.get("data")?;
    let GuardrailValue::Object(fields) = body else {
        return None;
    };
    if !matches!(fields.get("messages"), Some(GuardrailValue::Array(_))) {
        return None;
    }
    let collected = collect_request_texts(body, skip);
    if collected.is_empty() {
        return None;
    }
    Some(collected)
}
