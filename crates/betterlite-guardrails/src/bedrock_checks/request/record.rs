use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::{ChecksInvocation, RecordOutcome, RecordTimes};
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
};

struct SilentSpan;

impl GuardrailSpanSink for SilentSpan {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

static SILENT_SPAN: SilentSpan = SilentSpan;

pub(super) fn write_record(
    invocation: &mut ChecksInvocation<'_>,
    event_type: GuardrailEventHooks,
    times: RecordTimes,
    outcome: RecordOutcome,
) {
    let mut context = GuardrailRecordContext {
        base: invocation.base,
        request_data: &mut *invocation.request_data,
        log_target: invocation.log_target.as_deref_mut(),
        sink: &SILENT_SPAN,
    };
    let record = GuardrailRecordInput {
        guardrail_response: outcome.response,
        guardrail_status: outcome.status,
        start_time: Some(epoch_seconds(times.started)),
        end_time: Some(epoch_seconds(times.finished)),
        duration: Some(
            times
                .finished
                .duration_since(times.started)
                .unwrap_or_default()
                .as_secs_f64(),
        ),
        masked_entity_count: None,
        guardrail_provider: Some(invocation.guardrail.guardrail_provider.to_owned()),
        event_type: Some(event_type),
        tracing_detail: outcome.tracing_detail,
    };
    if let Err(error) = add_guardrail_record(&mut context, record) {
        tracing::warn!(
            "Bedrock InvokeGuardrailChecks: the guardrail record was not written: {error}"
        );
    }
}

fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
