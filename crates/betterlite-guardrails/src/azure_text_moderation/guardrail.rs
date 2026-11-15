use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use crate::azure_content_safety::api_version::resolve_api_version;
use crate::azure_content_safety::base::{AzureGuardrailBase, DEFAULT_TIMEOUT};
use crate::azure_content_safety::constants::MAX_TEXT_LENGTH;
use crate::azure_content_safety::legacy_wire::{ModelResponse, TextCategory};
use crate::azure_content_safety::text_splitting::split_text_by_words;
use crate::azure_content_safety::user_prompt::get_user_prompt_from_request;

use super::config::{DEFAULT_CATEGORIES, DEFAULT_SEVERITY_THRESHOLD};
use super::error::{AzureTextModerationError, ViolationDetail};
use super::wire::{AnalyzeRequest, AnalyzeResponse, BodyOptionalParams, CategoryAnalysis};
use super::{AzureTextModerationGuardrail, AzureTextModerationParams};

const ANALYZE_TEXT_PATH: &str = "text:analyze";
const RESPONSES_CALL_TYPES: [&str; 2] = ["responses", "aresponses"];

#[derive(Serialize)]
struct StreamFrame {
    error: ViolationDetail,
}

impl AzureTextModerationGuardrail {
    #[must_use]
    pub fn new(params: AzureTextModerationParams, client: reqwest::Client) -> Self {
        let optional_params = params.optional_params;
        Self {
            base: AzureGuardrailBase {
                async_handler: client,
                api_key: params.api_key,
                api_base: params.api_base,
                api_version: resolve_api_version(params.api_version.as_deref()).to_owned(),
                timeout: params.timeout.unwrap_or(DEFAULT_TIMEOUT),
            },
            severity_threshold: optional_params.severity_threshold,
            severity_threshold_by_category: optional_params.severity_threshold_by_category,
            optional_params_request_body: BodyOptionalParams {
                categories: optional_params
                    .categories
                    .unwrap_or_else(|| DEFAULT_CATEGORIES.to_vec()),
                blocklist_names: optional_params.blocklist_names,
                halt_on_blocklist_hit: optional_params.halt_on_blocklist_hit.unwrap_or_default(),
                output_type: optional_params.output_type.unwrap_or_default(),
            },
        }
    }

    pub(crate) async fn scan_texts(
        &self,
        texts: &[String],
    ) -> Result<(), AzureTextModerationError> {
        for text in texts.iter().filter(|text| !text.is_empty()) {
            self.async_make_request(text).await?;
        }
        Ok(())
    }

    pub async fn async_make_request(
        &self,
        text: &str,
    ) -> Result<AnalyzeResponse, AzureTextModerationError> {
        let chunks = split_text_by_words(text, MAX_TEXT_LENGTH);
        let mut last_response = None;
        for chunk in &chunks {
            let request = AnalyzeRequest {
                text: chunk,
                categories: &self.optional_params_request_body.categories,
                blocklist_names: self.optional_params_request_body.blocklist_names.as_deref(),
                halt_on_blocklist_hit: self.optional_params_request_body.halt_on_blocklist_hit,
                output_type: self.optional_params_request_body.output_type,
            };
            let response: AnalyzeResponse = self
                .base
                .post_to_content_safety(ANALYZE_TEXT_PATH, &request)
                .await?;
            // LiteLLM checks every chunk, so a violation in an early one still blocks the request
            if let Err(error) = self.check_severity_threshold(&response) {
                tracing::warn!(
                    length = chunk.chars().count(),
                    "Azure Text Moderation: violation detected in a chunk of text"
                );
                return Err(error);
            }
            last_response = Some(response);
        }
        last_response.ok_or(AzureTextModerationError::NoChunks)
    }

    // LiteLLM checks a category map first, then a global threshold, then the default.
    pub fn check_severity_threshold(
        &self,
        response: &AnalyzeResponse,
    ) -> Result<(), AzureTextModerationError> {
        if let Some(thresholds) = self.severity_threshold_by_category.as_ref() {
            for analysis in &response.categories_analysis {
                check_category_entry(thresholds, analysis)?;
            }
        }
        if let Some(threshold) = self.severity_threshold {
            for analysis in &response.categories_analysis {
                violation_at(analysis, threshold)?;
            }
        }
        if self.severity_threshold.is_none() && self.severity_threshold_by_category.is_none() {
            for analysis in &response.categories_analysis {
                violation_at(analysis, DEFAULT_SEVERITY_THRESHOLD)?;
            }
        }
        Ok(())
    }

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
                            GuardrailValue::Null
                            | GuardrailValue::Boolean(_)
                            | GuardrailValue::SignedInteger(_)
                            | GuardrailValue::UnsignedInteger(_)
                            | GuardrailValue::Decimal(_)
                            | GuardrailValue::Array(_)
                            | GuardrailValue::Object(_) => None,
                        })
                    }
                    GuardrailValue::Null
                    | GuardrailValue::Boolean(_)
                    | GuardrailValue::SignedInteger(_)
                    | GuardrailValue::UnsignedInteger(_)
                    | GuardrailValue::Decimal(_)
                    | GuardrailValue::Text(_)
                    | GuardrailValue::Array(_) => None,
                })
                .filter(|text| !text.is_empty())
                .collect();
            parts.join("\n")
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Object(_) => String::new(),
    }
}

// A response category outside the four names has no map entry and is not checked.
fn check_category_entry(
    thresholds: &BTreeMap<TextCategory, i64>,
    analysis: &CategoryAnalysis,
) -> Result<(), AzureTextModerationError> {
    let lookup = TextCategory::from_name(&analysis.category)
        .and_then(|category| thresholds.get(&category).copied());
    if let Some(threshold) = lookup {
        violation_at(analysis, threshold)?;
    }
    Ok(())
}

fn violation_at(
    analysis: &CategoryAnalysis,
    threshold: i64,
) -> Result<(), AzureTextModerationError> {
    if analysis.severity < threshold {
        return Ok(());
    }
    Err(AzureTextModerationError::Violation {
        category: analysis.category.clone(),
        threshold,
        severity: analysis.severity,
    })
}
