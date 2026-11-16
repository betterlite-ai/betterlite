use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::data::{Data, MESSAGES, parse_items};
use super::error::HookDataError;
use crate::prisma_airs_extraction::select_last_eligible_index;
use crate::prisma_airs_masking::{RequestMessage, mask_messages};

// Mirrors LiteLLM's fn _apply_masking_to_messages: only the masked message's content changes, its other keys stay.
pub fn apply_masking_to_messages(data: &mut Data, masked_text: &str) -> Result<(), HookDataError> {
    let Some(value) = data.get(MESSAGES) else {
        return Ok(());
    };
    let GuardrailValue::Array(values) = value else {
        return Err(HookDataError::NotList { field: MESSAGES });
    };
    let mut messages: Vec<RequestMessage> = parse_items(values, MESSAGES)?;
    let index = select_last_eligible_index(&messages);
    mask_messages(&mut messages, masked_text)
        .map_err(|source| HookDataError::Masking { source })?;
    let Some(index) = index else {
        return Ok(());
    };
    let Some(content) = messages
        .get(index)
        .and_then(|message| message.content.as_ref())
    else {
        return Ok(());
    };
    let encoded =
        serde_json::to_value(content).map_err(|source| HookDataError::Encoding { source })?;
    let content: GuardrailValue =
        serde_json::from_value(encoded).map_err(|source| HookDataError::Encoding { source })?;
    let Some(GuardrailValue::Array(values)) = data.get_mut(MESSAGES) else {
        return Ok(());
    };
    let Some(GuardrailValue::Object(message)) = values.get_mut(index) else {
        return Ok(());
    };
    message.insert("content".to_owned(), content);
    Ok(())
}
