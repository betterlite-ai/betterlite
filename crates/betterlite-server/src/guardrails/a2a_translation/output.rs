use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

use super::A2aTranslationError;
use super::paths::{self, TextPlacement};
use crate::guardrails::response_context::add_response_context;

pub async fn process_output_response<G>(
    response: &mut GuardrailValue,
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
    user_api_key_dict: Option<&BTreeMap<String, GuardrailValue>>,
    request_data: Option<&mut BTreeMap<String, GuardrailValue>>,
) -> Result<(), A2aTranslationError>
where
    G: GuardrailContract,
{
    let Some(placements) = scannable_placements(response) else {
        return Ok(());
    };
    let mut fresh_request_data = BTreeMap::new();
    let prepared_request_data = request_data.unwrap_or(&mut fresh_request_data);
    add_response_context(prepared_request_data, response, user_api_key_dict);
    let inputs = GuardrailApiInputs {
        texts: Some(
            placements
                .iter()
                .map(|placement| placement.text.clone())
                .collect(),
        ),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
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
    let Some(returned) = guardrailed.texts else {
        return Ok(());
    };
    write_back(response, &placements, &returned);
    Ok(())
}

fn scannable_placements(response: &GuardrailValue) -> Option<Vec<TextPlacement>> {
    let GuardrailValue::Object(fields) = response else {
        tracing::warn!(
            "A2A: Unknown response type {}, skipping guardrail",
            value_type_name(response)
        );
        return None;
    };
    let Some(GuardrailValue::Object(result)) = fields.get("result") else {
        tracing::debug!("A2A: No result in response, skipping guardrail");
        return None;
    };
    if result.is_empty() {
        tracing::debug!("A2A: No result in response, skipping guardrail");
        return None;
    }
    let placements = paths::extract_texts_from_result(result);
    if placements.is_empty() {
        tracing::debug!("A2A: No text content in response");
        return None;
    }
    Some(placements)
}

fn write_back(response: &mut GuardrailValue, placements: &[TextPlacement], returned: &[String]) {
    if returned.len() != placements.len() {
        return;
    }
    let GuardrailValue::Object(updated_fields) = response else {
        return;
    };
    let Some(GuardrailValue::Object(updated_result)) = updated_fields.get_mut("result") else {
        return;
    };
    for (placement, text) in placements.iter().zip(returned.iter()) {
        paths::apply_text_to_path(updated_result, &placement.path, placement.part_index, text);
    }
    tracing::debug!("A2A: Processed output response");
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
