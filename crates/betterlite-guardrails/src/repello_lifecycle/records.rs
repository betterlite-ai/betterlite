use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_records::invocation::GuardrailInvocation;
use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus,
};

use super::Data;
use super::RepelloAILifecycle;
use super::hooks::HookContext;

// One scan's values, as LiteLLM's finally block records them
pub(super) struct ScanRun {
    pub(super) event_type: GuardrailEventHooks,
    pub(super) response: GuardrailValue,
    pub(super) status: GuardrailStatus,
    pub(super) started: SystemTime,
    pub(super) finished: SystemTime,
}

pub(super) fn record_scan(
    guardrail: &RepelloAILifecycle,
    request_data: &mut Data,
    context: &mut HookContext<'_>,
    run: ScanRun,
) {
    let start_time = run
        .started
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let end_time = run
        .finished
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let duration = run.finished.duration_since(run.started).unwrap_or_default();
    let input = GuardrailRecordInput {
        guardrail_response: run.response,
        guardrail_status: run.status,
        start_time: Some(start_time),
        end_time: Some(end_time),
        duration: Some(duration.as_secs_f64()),
        masked_entity_count: Some(BTreeMap::new()),
        guardrail_provider: None,
        event_type: Some(run.event_type),
        tracing_detail: None,
    };
    let invocation = &mut *context.invocation;
    let log_target = context.log_target.as_deref_mut();
    let mut record = GuardrailRecordContext {
        base: &guardrail.base,
        request_data,
        log_target,
        sink: context.sink,
    };
    if let Err(error) = invocation.record_own(&mut record, input) {
        tracing::warn!("RepelloAI Argus guardrail record was not written: {error}");
    }
}
