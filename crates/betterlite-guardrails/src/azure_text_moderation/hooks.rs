use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use crate::azure_content_safety::legacy_wire::ModelResponse;
use crate::azure_content_safety::user_prompt::get_user_prompt_from_request;

use super::AzureTextModerationGuardrail;
use super::error::{AzureTextModerationError, ViolationDetail};

const RESPONSES_CALL_TYPES: [&str; 2] = ["responses", "aresponses"];

#[derive(Serialize)]
struct StreamFrame {
    error: ViolationDetail,
}

impl AzureTextModerationGuardrail {
    pub async fn async_pre_call_hook(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        call_type: &str,
    ) -> Result<Option<BTreeMap<String, GuardrailValue>>, AzureTextModerationError> {
        if !RESPONSES_CALL_TYPES.contains(&call_type)
            && matches!(data.get("messages"), None | Some(GuardrailValue::Null))
        {
            tracing::warn!("Azure Text Moderation: not running guardrail. No messages in data");
            return Ok(Some(data.clone()));
        }
        // None covers the empty prompt LiteLLM's `if user_prompt:` sends to the warning branch.
        let Some(user_prompt) = get_user_prompt_from_request(data, call_type)? else {
            tracing::warn!("Azure Text Moderation: No text found");
            return Ok(None);
        };
        self.async_make_request(&user_prompt).await?;
        Ok(None)
    }

    pub async fn async_post_call_success_hook(
        &self,
        response: Option<&ModelResponse>,
    ) -> Result<(), AzureTextModerationError> {
        let Some(model_response) = response else {
            tracing::debug!(
                "Azure Text Moderation: skipping a response that is not a ModelResponse"
            );
            return Ok(());
        };
        for choice in &model_response.choices {
            let Some(message) = &choice.message else {
                continue;
            };
            let Some(content) = &message.content else {
                continue;
            };
            let text = message_content_to_text(content);
            if text.is_empty() {
                continue;
            }
            self.async_make_request(&text).await?;
        }
        Ok(())
    }

    pub async fn async_post_call_streaming_hook(
        &self,
        response: &str,
    ) -> Result<String, AzureTextModerationError> {
        if response.is_empty() {
            return Ok(response.to_owned());
        }
        match self.async_make_request(response).await {
            Ok(_) => Ok(response.to_owned()),
            // LiteLLM answers a violation in a chunk with a data frame instead of raising
            Err(error @ AzureTextModerationError::Violation { .. }) => {
                let frame = StreamFrame {
                    error: ViolationDetail {
                        error: error.to_string(),
                    },
                };
                let json = serde_json::to_string(&frame)
                    .map_err(|source| AzureTextModerationError::Frame { source })?;
                Ok(format!("data: {json}\n\n"))
            }
            Err(error) => Err(error),
        }
    }
}

// LiteLLM's _message_content_to_text: a string as it is, or the text blocks of a content list.
fn message_content_to_text(content: &GuardrailValue) -> String {
    match content {
        GuardrailValue::Text(text) => text.clone(),
        GuardrailValue::Array(items) => {
            let parts: Vec<&str> = items
                .iter()
                .filter_map(|item| match item {
                    GuardrailValue::Object(fields) => {
                        fields.get("text").and_then(|value| match value {
                            GuardrailValue::Text(text) => Some(text.as_str()),
                            _ => None,
                        })
                    }
                    _ => None,
                })
                .filter(|text| !text.is_empty())
                .collect();
            parts.join("\n")
        }
        _ => String::new(),
    }
}
