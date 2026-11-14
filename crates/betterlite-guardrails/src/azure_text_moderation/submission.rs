use crate::azure_content_safety::constants::MAX_TEXT_LENGTH;
use crate::azure_content_safety::text_splitting::split_text_by_words;

use super::AzureTextModerationGuardrail;
use super::error::AzureTextModerationError;
use super::wire::{AnalyzeRequest, AnalyzeResponse};

const ANALYZE_TEXT_PATH: &str = "text:analyze";

impl AzureTextModerationGuardrail {
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
}
