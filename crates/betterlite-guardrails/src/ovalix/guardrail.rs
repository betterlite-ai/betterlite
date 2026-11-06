use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::ExposeSecret as _;

use super::checkpoint::{
    CheckpointData, CheckpointError, CheckpointRequest, CheckpointResponse, DataType,
};
use super::error::OvalixError;
use super::exception_status::{GuardrailRaisedException, OvalixBlockedException};
use super::session_identity::{self, Clock, RequestData};
use super::{
    BLOCKED_ACTION_TYPE, BLOCKED_FALLBACK_MESSAGE, CHECKPOINT_PATH, InputType, Inputs,
    MAX_RESPONSE_BYTES, OvalixGuardrail,
};

impl<C: Clock> OvalixGuardrail<C> {
    pub fn prepare_headers(&self) -> Result<HeaderMap, CheckpointError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let bearer = format!("Bearer {}", self.settings.tracker_api_key.expose_secret());
        let mut authorization = HeaderValue::from_str(&bearer)
            .map_err(|source| CheckpointError::InvalidHeader { source })?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        Ok(headers)
    }

    pub async fn call_checkpoint(
        &self,
        content: &str,
        checkpoint_id: &str,
        actor: &str,
        session_id: &str,
    ) -> Result<CheckpointResponse, CheckpointError> {
        let request = CheckpointRequest {
            application_id: self.settings.application_id.clone(),
            checkpoint_id: checkpoint_id.to_owned(),
            actor: actor.to_owned(),
            session_id: session_id.to_owned(),
            data_type: DataType::Text,
            data: CheckpointData {
                content: content.to_owned(),
            },
        };
        let url = format!("{}{CHECKPOINT_PATH}", self.settings.tracker_api_base);
        let response = self
            .client
            .post(&url)
            .headers(self.prepare_headers()?)
            .json(&request)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(CheckpointError::Status { status });
        }
        let body = read_capped_body(response).await?;
        serde_json::from_slice(&body).map_err(|source| CheckpointError::ResponseJson { source })
    }

    pub async fn apply_guardrail(
        &self,
        mut inputs: Inputs,
        request_data: &RequestData,
        input_type: InputType,
    ) -> Result<Inputs, OvalixError> {
        let selected = match input_type {
            InputType::Request => self.settings.pre_checkpoint_id.as_deref(),
            InputType::Response => self.settings.post_checkpoint_id.as_deref(),
        };
        let Some(checkpoint_id) = selected else {
            return Ok(inputs);
        };
        if inputs.texts.is_empty() {
            return Ok(inputs);
        }
        let actor = session_identity::tracker_actor_id(request_data);
        let session_id = session_identity::session_id(
            request_data,
            self.clock.today(),
            &self.settings.application_id,
        );
        inputs.texts = self
            .generate_texts(&inputs.texts, checkpoint_id, &actor, &session_id)
            .await?;
        Ok(inputs)
    }

    async fn generate_texts(
        &self,
        texts: &[String],
        checkpoint_id: &str,
        actor: &str,
        session_id: &str,
    ) -> Result<Vec<String>, OvalixError> {
        let mut corrected = Vec::with_capacity(texts.len());
        let mut is_first_response = true;
        for text in texts.iter().rev() {
            let response = self
                .checkpoint_response(text, checkpoint_id, actor, session_id)
                .await?;
            corrected.insert(0, self.corrected_text(&response, text, is_first_response)?);
            is_first_response = false;
        }
        Ok(corrected)
    }

    async fn checkpoint_response(
        &self,
        text: &str,
        checkpoint_id: &str,
        actor: &str,
        session_id: &str,
    ) -> Result<CheckpointResponse, GuardrailRaisedException> {
        self.call_checkpoint(text, checkpoint_id, actor, session_id)
            .await
            .map_err(|source| {
                tracing::error!(
                    guardrail = %self.guardrail_name,
                    error = %source,
                    "the Ovalix checkpoint call failed"
                );
                GuardrailRaisedException {
                    guardrail_name: self.guardrail_name.clone(),
                    source,
                }
            })
    }

    fn corrected_text(
        &self,
        response: &CheckpointResponse,
        text: &str,
        is_first_response: bool,
    ) -> Result<String, OvalixError> {
        let corrected_content = response
            .modified_data
            .as_ref()
            .and_then(|data| data.content.as_deref());
        let blocked = response
            .action_type
            .as_deref()
            .is_some_and(|action| action.eq_ignore_ascii_case(BLOCKED_ACTION_TYPE));
        let blocking_message = corrected_content.unwrap_or(BLOCKED_FALLBACK_MESSAGE);
        if blocked && is_first_response {
            return Err(OvalixError::Blocked(OvalixBlockedException {
                guardrail_name: self.guardrail_name.clone(),
                message: blocking_message.to_owned(),
            }));
        }
        if blocked {
            return Ok(blocking_message.to_owned());
        }
        Ok(corrected_content.unwrap_or(text).to_owned())
    }
}

fn request_error(source: reqwest::Error) -> CheckpointError {
    if source.is_timeout() {
        CheckpointError::Timeout { source }
    } else {
        CheckpointError::Request { source }
    }
}

async fn read_capped_body(mut response: reqwest::Response) -> Result<Vec<u8>, CheckpointError> {
    let limit = MAX_RESPONSE_BYTES;
    if response
        .content_length()
        .is_some_and(|length| length > u64::try_from(limit).unwrap_or(u64::MAX))
    {
        return Err(CheckpointError::ResponseTooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(CheckpointError::ResponseTooLarge { limit });
        }
    }
    Ok(body)
}
