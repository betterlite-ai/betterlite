use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use secrecy::ExposeSecret as _;
use serde::Serialize;

use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus,
};
use crate::lakera_pii::wire::{LakeraResponse, Message};

use super::super::Data;
use super::super::error::LakeraV2Error;
use super::super::hooks::HookContext;
use super::LakeraV2Guardrail;

const GUARD_PATH: &str = "/v2/guard";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Serialize)]
struct GuardRequest<'request> {
    messages: &'request [Message],
    project_id: Option<&'request str>,
    payload: GuardrailFlag,
    breakdown: GuardrailFlag,
    metadata: Option<&'request BTreeMap<String, GuardrailValue>>,
    dev_info: GuardrailFlag,
}

impl LakeraV2Guardrail {
    pub async fn call_v2_guard(
        &self,
        messages: &[Message],
        data: &mut Data,
        event: GuardrailEventHooks,
        context: &mut HookContext<'_>,
    ) -> Result<(LakeraResponse, BTreeMap<String, u64>), LakeraV2Error> {
        let started_at = SystemTime::now();
        let result = self.post_guard(messages).await;
        let finished_at = SystemTime::now();
        let input = record_input(&result, event, started_at, finished_at)?;
        let sink = context.sink;
        let invocation = &mut *context.invocation;
        let log_target = context.log_target.as_deref_mut();
        let mut record = GuardrailRecordContext {
            base: &self.base,
            request_data: data,
            log_target,
            sink,
        };
        invocation
            .record_own(&mut record, input)
            .map_err(|source| LakeraV2Error::Record { source })?;
        result.map(|response| (response, BTreeMap::new()))
    }

    async fn post_guard(&self, messages: &[Message]) -> Result<LakeraResponse, LakeraV2Error> {
        let request = GuardRequest {
            messages,
            project_id: self.project_id.as_deref(),
            payload: self.flags.payload,
            breakdown: self.flags.breakdown,
            metadata: self.metadata.as_ref(),
            dev_info: self.flags.dev_info,
        };
        let mut builder = self
            .client
            .post(format!("{}{GUARD_PATH}", self.api_base))
            .bearer_auth(self.api_key.expose_secret())
            .json(&request);
        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }
        let mut response = builder
            .send()
            .await
            .map_err(|source| LakeraV2Error::Transport { source })?;
        let read = read_bounded(&mut response, MAX_RESPONSE_BYTES).await;
        let bytes = read.map_err(|error| match error {
            ReadError::TooLarge { limit } => LakeraV2Error::ResponseTooLarge { limit },
            ReadError::Failed(source) => LakeraV2Error::Transport { source },
        })?;
        serde_json::from_slice(&bytes).map_err(|source| LakeraV2Error::Decode { source })
    }
}

fn record_input(
    result: &Result<LakeraResponse, LakeraV2Error>,
    event: GuardrailEventHooks,
    started_at: SystemTime,
    finished_at: SystemTime,
) -> Result<GuardrailRecordInput, LakeraV2Error> {
    let (guardrail_response, guardrail_status) = match result {
        Ok(response) => (
            response_without_payload(response)?,
            GuardrailStatus::Success,
        ),
        Err(error) => (
            GuardrailValue::Text(error.to_string()),
            GuardrailStatus::GuardrailFailedToRespond,
        ),
    };
    let start_time = epoch_seconds(started_at);
    let end_time = epoch_seconds(finished_at);
    let duration = finished_at.duration_since(started_at).unwrap_or_default();
    Ok(GuardrailRecordInput {
        guardrail_response,
        guardrail_status,
        start_time: Some(start_time),
        end_time: Some(end_time),
        duration: Some(duration.as_secs_f64()),
        masked_entity_count: Some(BTreeMap::new()),
        guardrail_provider: None,
        event_type: Some(event),
        tracing_detail: None,
    })
}

fn epoch_seconds(at: SystemTime) -> f64 {
    at.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// LiteLLM pops payload because it holds the PII spans; a response without it keeps working here
fn response_without_payload(response: &LakeraResponse) -> Result<GuardrailValue, LakeraV2Error> {
    let mut value =
        serde_json::to_value(response).map_err(|source| LakeraV2Error::Json { source })?;
    if let Some(fields) = value.as_object_mut() {
        fields.remove("payload");
    }
    serde_json::from_value(value).map_err(|source| LakeraV2Error::Json { source })
}
