use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus,
};

pub struct GuardrailFailureLog<'call> {
    pub message: String,
    pub status_code: Option<u16>,
    pub started_at: SystemTime,
    pub finished_at: SystemTime,
    pub request_data: &'call mut BTreeMap<String, GuardrailValue>,
    pub log_target: Option<&'call mut GuardrailLogTarget>,
}

// LiteLLM's _log_guardrail_failure: a failed-to-respond record, then the error log line with its status code
pub fn log_guardrail_failure(base: &GuardrailBase, log: GuardrailFailureLog<'_>) {
    let GuardrailFailureLog {
        message,
        status_code,
        started_at,
        finished_at,
        request_data,
        log_target,
    } = log;
    let mut context = GuardrailRecordContext {
        base,
        request_data,
        log_target,
        sink: &NO_SPAN,
    };
    let record = GuardrailRecordInput {
        guardrail_response: GuardrailValue::Text(message.clone()),
        guardrail_status: GuardrailStatus::GuardrailFailedToRespond,
        start_time: Some(epoch_seconds(started_at)),
        end_time: Some(epoch_seconds(finished_at)),
        duration: Some(
            finished_at
                .duration_since(started_at)
                .unwrap_or_default()
                .as_secs_f64(),
        ),
        masked_entity_count: None,
        guardrail_provider: Some("grayswan".to_owned()),
        event_type: None,
        tracing_detail: None,
    };
    if let Err(error) = add_guardrail_record(&mut context, record) {
        tracing::warn!("Gray Swan Guardrail: failed to log guardrail failure: {error}");
    }
    if let Some(code) = status_code {
        tracing::error!("Gray Swan Guardrail: API request failed (status_code={code}): {message}");
    } else {
        tracing::error!("Gray Swan Guardrail: API request failed: {message}");
    }
}

fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

struct NoSpan;

impl GuardrailSpanSink for NoSpan {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

static NO_SPAN: NoSpan = NoSpan;
