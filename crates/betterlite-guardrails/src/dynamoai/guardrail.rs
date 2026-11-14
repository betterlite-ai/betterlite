use betterlite_config::sections::dynamoai_moderation::Configured;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::ExposeSecret as _;

use super::decisions::{GuardrailStatus, create_error_message, determine_status, process_response};
use super::error::DynamoAiError;
use super::hooks::{PostCallOutcome, RequestData, ResponseChoice, extracted_message};
use super::wire::{ANALYZE_PATH, DynamoAiRequest, DynamoAiResponse};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub struct DynamoAiGuardrail {
    pub settings: Configured,
    pub client: reqwest::Client,
}

impl DynamoAiGuardrail {
    pub fn prepare_headers(&self) -> Result<HeaderMap, DynamoAiError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {}", self.settings.api_key.expose_secret()))
                .map_err(|source| DynamoAiError::InvalidHeader { source })?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        Ok(headers)
    }

    #[must_use]
    pub fn prepare_payload(&self, messages: &[GuardrailValue]) -> DynamoAiRequest {
        DynamoAiRequest {
            messages: messages.to_vec(),
            policy_ids: (!self.settings.policy_ids.is_empty())
                .then(|| self.settings.policy_ids.clone()),
            model_id: (!self.settings.model_id.is_empty()).then(|| self.settings.model_id.clone()),
        }
    }

    pub async fn call_moderation(
        &self,
        messages: &[GuardrailValue],
    ) -> Result<DynamoAiResponse, DynamoAiError> {
        let url = format!("{}{ANALYZE_PATH}", self.settings.api_base);
        let payload = self.prepare_payload(messages);
        let mut response = self
            .client
            .post(url)
            .headers(self.prepare_headers()?)
            .json(&payload)
            .timeout(self.settings.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(DynamoAiError::HttpStatus { status });
        }
        let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(read_error)?;
        serde_json::from_slice(&body).map_err(|source| DynamoAiError::ResponseJson { source })
    }

    pub async fn async_pre_call_hook(
        &self,
        data: &RequestData,
    ) -> Result<GuardrailStatus, DynamoAiError> {
        if data.messages.is_empty() {
            return Ok(GuardrailStatus::Success);
        }
        self.inspect_messages(&data.messages).await
    }

    pub async fn async_moderation_hook(
        &self,
        messages: &[GuardrailValue],
    ) -> Result<GuardrailStatus, DynamoAiError> {
        if messages.is_empty() {
            Ok(GuardrailStatus::Success)
        } else {
            self.inspect_messages(messages).await
        }
    }

    pub async fn async_post_call_success_hook(
        &self,
        choices: &[ResponseChoice],
    ) -> Result<PostCallOutcome, DynamoAiError> {
        let messages: Vec<GuardrailValue> = choices.iter().filter_map(extracted_message).collect();
        if messages.is_empty() {
            return Ok(PostCallOutcome::NoText);
        }
        let status = self.inspect_messages(&messages).await?;
        Ok(PostCallOutcome::Inspected(status))
    }

    async fn inspect_messages(
        &self,
        messages: &[GuardrailValue],
    ) -> Result<GuardrailStatus, DynamoAiError> {
        let response = self.call_moderation(messages).await?;
        let status = determine_status(&response);
        let processed = process_response(&response);
        if processed.violations_detected.is_empty() {
            return Ok(status);
        }
        Err(DynamoAiError::Blocked {
            message: create_error_message(&processed),
        })
    }
}

fn request_error(source: reqwest::Error) -> DynamoAiError {
    if source.is_timeout() {
        DynamoAiError::Timeout { source }
    } else {
        DynamoAiError::Request { source }
    }
}

fn read_error(error: ReadError) -> DynamoAiError {
    match error {
        ReadError::TooLarge { limit } => DynamoAiError::ResponseTooLarge { limit },
        ReadError::Failed(source) => request_error(source),
    }
}
