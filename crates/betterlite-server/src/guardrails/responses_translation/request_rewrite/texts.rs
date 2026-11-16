use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;

use super::ExtractedInputs;
use super::error::RequestRewriteError;

pub fn apply_guardrailed_texts(
    data: &mut Object,
    input_data: &GuardrailValue,
    extracted: &ExtractedInputs,
    answer: &GuardrailApiInputs,
    guardrail_name: Option<&str>,
) -> Result<(), RequestRewriteError> {
    let Some(returned) = answer.texts.as_deref().filter(|texts| !texts.is_empty()) else {
        return Ok(());
    };
    let offset = usize::from(extracted.instructions.is_some());
    let is_string = matches!(input_data, GuardrailValue::Text(_));
    let expected = if is_string {
        1
    } else {
        extracted.task_mappings.len()
    };
    if returned.len() != offset.saturating_add(expected) {
        return Err(RequestRewriteError::Unappliable {
            source: UnappliableRequestRewrite::new(guardrail_name),
        });
    }
    if let Some(instructions) = returned.first().filter(|_| offset == 1) {
        data.insert(
            "instructions".to_owned(),
            GuardrailValue::Text(instructions.clone()),
        );
    }
    let input_texts = returned.get(offset..).unwrap_or_default();
    if is_string {
        if let Some(text) = input_texts.first() {
            data.insert("input".to_owned(), GuardrailValue::Text(text.clone()));
        }
        return Ok(());
    }
    if let Some(GuardrailValue::Array(messages)) = data.get_mut("input") {
        apply_guardrail_responses_to_input(messages, input_texts, &extracted.task_mappings);
    }
    Ok(())
}

// The two lists pair up to the shorter, and the caller has already checked they agree
pub fn apply_guardrail_responses_to_input(
    messages: &mut [GuardrailValue],
    responses: &[String],
    task_mappings: &[(usize, Option<usize>)],
) {
    for (response, (message_index, content_index)) in responses.iter().zip(task_mappings) {
        let Some(GuardrailValue::Object(message)) = messages.get_mut(*message_index) else {
            continue;
        };
        let Some(content) = message.get_mut("content") else {
            continue;
        };
        match content {
            GuardrailValue::Text(text) if content_index.is_none() => text.clone_from(response),
            GuardrailValue::Array(entries) => {
                if let Some(index) = content_index
                    && let Some(GuardrailValue::Object(entry)) = entries.get_mut(*index)
                {
                    entry.insert("text".to_owned(), GuardrailValue::Text(response.clone()));
                }
            }
            GuardrailValue::Text(_)
            | GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Object(_) => {}
        }
    }
}
