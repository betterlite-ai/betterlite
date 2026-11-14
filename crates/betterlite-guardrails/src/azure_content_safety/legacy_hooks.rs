use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::AzureContentSafetyError;
use super::legacy::LegacyAzureContentSafety;
use super::legacy_wire::ModelResponse;
use crate::content_inspection::{TEXT_CONTENT_CALL_TYPES, iter_message_text};

pub async fn async_pre_call_hook(
    hook: &LegacyAzureContentSafety,
    data: &BTreeMap<String, GuardrailValue>,
    call_type: &str,
) -> Result<(), AzureContentSafetyError> {
    if !TEXT_CONTENT_CALL_TYPES.contains(&call_type) {
        return Ok(());
    }
    for text in iter_message_text(data) {
        match hook.test_violation(&text, Some("input")).await {
            Ok(()) => {}
            Err(error @ AzureContentSafetyError::Violation { .. }) => return Err(error),
            Err(error) => {
                // LiteLLM logs the failure and leaves the try block, so later text is never scanned.
                tracing::error!(%error, "azure_content_safety pre-call hook exception");
                return Ok(());
            }
        }
    }
    Ok(())
}

pub async fn async_post_call_success_hook(
    hook: &LegacyAzureContentSafety,
    response: Option<&ModelResponse>,
) -> Result<(), AzureContentSafetyError> {
    let Some(model_response) = response else {
        return Ok(());
    };
    for choice in &model_response.choices {
        let content = choice
            .message
            .as_ref()
            .and_then(|message| message.content.as_ref());
        let Some(GuardrailValue::Text(text)) = content else {
            continue;
        };
        hook.test_violation(text, Some("output")).await?;
    }
    Ok(())
}
