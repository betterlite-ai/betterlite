use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus,
};

use super::error::EnkryptAiError;
use super::{EnkryptAiGuardrail, GUARDRAIL_PROVIDER};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    pub(crate) started: SystemTime,
    pub(crate) finished: SystemTime,
}

pub(crate) struct RecordRequest<'request> {
    pub(crate) guardrail: &'request EnkryptAiGuardrail,
    pub(crate) request_data: &'request mut BTreeMap<String, GuardrailValue>,
    pub(crate) log_target: Option<&'request mut GuardrailLogTarget>,
    pub(crate) response: GuardrailValue,
    pub(crate) status: GuardrailStatus,
    pub(crate) timing: Timing,
}

pub(crate) fn record(request: RecordRequest<'_>) -> Result<(), EnkryptAiError> {
    let RecordRequest {
        guardrail,
        request_data,
        log_target,
        response,
        status,
        timing,
    } = request;
    let mut context = GuardrailRecordContext {
        base: &guardrail.base,
        request_data,
        log_target,
        sink: guardrail.sink.as_ref(),
    };
    let input = GuardrailRecordInput {
        guardrail_response: response,
        guardrail_status: status,
        start_time: Some(seconds(timing.started)),
        end_time: Some(seconds(timing.finished)),
        duration: Some(
            timing
                .finished
                .duration_since(timing.started)
                .unwrap_or_default()
                .as_secs_f64(),
        ),
        masked_entity_count: None,
        guardrail_provider: Some(GUARDRAIL_PROVIDER.to_owned()),
        event_type: None,
        tracing_detail: None,
    };
    add_guardrail_record(&mut context, input).map_err(|source| EnkryptAiError::Record { source })
}

fn seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
