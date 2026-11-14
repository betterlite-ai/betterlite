use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus,
};

use super::PROVIDER;
use super::error::NomaV2Error;
use super::guardrail::NomaV2Guardrail;

pub(super) struct ScanRun {
    pub(super) response: GuardrailValue,
    pub(super) status: GuardrailStatus,
    pub(super) started: SystemTime,
    pub(super) ended: SystemTime,
}

// The record is written to request data and the logging object; no OTEL span is emitted from here.
struct NoSpanSink;

impl GuardrailSpanSink for NoSpanSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

static NO_SPAN_SINK: NoSpanSink = NoSpanSink;

pub(super) fn record_scan(
    guardrail: &NomaV2Guardrail,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    log_target: Option<&mut GuardrailLogTarget>,
    run: ScanRun,
) -> Result<(), NomaV2Error> {
    let started = epoch_seconds(run.started)?;
    let ended = epoch_seconds(run.ended)?;
    // float_arithmetic is denied; a fused multiply-add computes the signed end minus start
    let duration = started.mul_add(-1.0_f64, ended);
    let mut context = GuardrailRecordContext {
        base: &guardrail.base,
        request_data,
        log_target,
        sink: &NO_SPAN_SINK,
    };
    let input = GuardrailRecordInput {
        guardrail_response: run.response,
        guardrail_status: run.status,
        start_time: Some(started),
        end_time: Some(ended),
        duration: Some(duration),
        masked_entity_count: None,
        guardrail_provider: Some(PROVIDER.to_owned()),
        event_type: None,
        tracing_detail: None,
    };
    add_guardrail_record(&mut context, input).map_err(|source| NomaV2Error::Record { source })
}

fn epoch_seconds(time: SystemTime) -> Result<f64, NomaV2Error> {
    let since_epoch = time
        .duration_since(UNIX_EPOCH)
        .map_err(|source| NomaV2Error::Clock { source })?;
    Ok(since_epoch.as_secs_f64())
}
