use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::GuardrailSettings;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus,
};

use super::error::NomaError;
use super::wire::ScanResponse;
use super::{NomaScanPolicy, PROVIDER};

#[derive(Debug, Clone, Copy)]
pub(super) struct ScanTiming {
    pub(super) started: SystemTime,
    pub(super) ended: SystemTime,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct GuardrailRun<'run> {
    pub(super) response: &'run ScanResponse,
    pub(super) status: GuardrailStatus,
    pub(super) event_type: Option<GuardrailEventHooks>,
    pub(super) timing: ScanTiming,
}

// A scan writes its own record, so no OTEL span is emitted from here.
struct NoSpanSink;

impl GuardrailSpanSink for NoSpanSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

static NO_SPAN_SINK: NoSpanSink = NoSpanSink;

fn epoch_seconds(time: SystemTime) -> Result<f64, NomaError> {
    let since_epoch = time
        .duration_since(UNIX_EPOCH)
        .map_err(|source| NomaError::Clock { source })?;
    Ok(since_epoch.as_secs_f64())
}

pub(super) fn record_scan(
    policy: &NomaScanPolicy,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    run: &GuardrailRun<'_>,
) -> Result<(), NomaError> {
    let mut base = GuardrailBase::new(GuardrailSettings::default(), true)
        .map_err(|source| NomaError::RecordBase { source })?;
    base.settings
        .guardrail_name
        .clone_from(&policy.settings.guardrail_name);
    base.settings
        .event_hook
        .clone_from(&policy.settings.event_hook);
    let serialized =
        serde_json::to_value(run.response).map_err(|source| NomaError::RecordJson { source })?;
    let guardrail_response =
        serde_json::from_value(serialized).map_err(|source| NomaError::RecordJson { source })?;
    let duration = run
        .timing
        .ended
        .duration_since(run.timing.started)
        .unwrap_or_default();
    let mut context = GuardrailRecordContext {
        base: &base,
        request_data,
        log_target: None,
        sink: &NO_SPAN_SINK,
    };
    let input = GuardrailRecordInput {
        guardrail_response,
        guardrail_status: run.status,
        start_time: Some(epoch_seconds(run.timing.started)?),
        end_time: Some(epoch_seconds(run.timing.ended)?),
        duration: Some(duration.as_secs_f64()),
        masked_entity_count: None,
        guardrail_provider: Some(PROVIDER.to_owned()),
        event_type: run.event_type,
        tracing_detail: None,
    };
    add_guardrail_record(&mut context, input).map_err(|source| NomaError::Record { source })
}
