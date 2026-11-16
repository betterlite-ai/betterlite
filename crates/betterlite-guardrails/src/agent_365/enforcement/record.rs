use std::collections::BTreeMap;
use std::ops::{Div as _, Mul as _};
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::agent_365::enforcement_types::Verdict;
use crate::agent_365_tokens::Agent365Guardrail;
use crate::guardrail_metadata::RequestData;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus,
};

pub(super) const GUARDRAIL_PROVIDER: &str = "agent_365";
const LATENCY_MILLIS_PER_SECOND: f64 = 1000.0;
const LATENCY_TENTHS: f64 = 10.0;

// The arguments of LiteLLM's _record_verdict; a None latency leaves out both latency_ms and duration
#[derive(Debug, Clone, Copy)]
pub(super) struct VerdictRecord<'record> {
    pub verdict: Verdict,
    pub defender_status: Option<&'record str>,
    pub correlation_id: Option<&'record str>,
    pub latency: Option<Duration>,
    pub reason: Option<&'record str>,
}

struct NoSpan;

impl GuardrailSpanSink for NoSpan {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

// A record that cannot be written is logged, so it never decides the tool call
pub(super) fn record_verdict(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    record: &VerdictRecord<'_>,
) {
    let mut context = GuardrailRecordContext {
        base: &guardrail.base,
        request_data: data,
        log_target: None,
        sink: &NoSpan,
    };
    let input = GuardrailRecordInput {
        guardrail_response: payload_for(record),
        guardrail_status: status_for(record.verdict),
        start_time: None,
        end_time: None,
        duration: record.latency.map(|latency| latency.as_secs_f64()),
        masked_entity_count: None,
        guardrail_provider: Some(GUARDRAIL_PROVIDER.to_owned()),
        event_type: Some(GuardrailEventHooks::PreMcpCall),
        tracing_detail: None,
    };
    if let Err(error) = add_guardrail_record(&mut context, input) {
        tracing::warn!(%error, "Agent 365: guardrail record could not be written");
    }
}

// LiteLLM's _record_verdict payload, with latency_ms rounded to one decimal
fn payload_for(record: &VerdictRecord<'_>) -> GuardrailValue {
    let mut payload = BTreeMap::new();
    payload.insert(
        "verdict".to_owned(),
        GuardrailValue::Text(record.verdict.name().to_owned()),
    );
    if let Some(defender_status) = record.defender_status {
        payload.insert(
            "defender_status".to_owned(),
            GuardrailValue::Text(defender_status.to_owned()),
        );
    }
    if let Some(correlation_id) = record.correlation_id {
        payload.insert(
            "correlation_id".to_owned(),
            GuardrailValue::Text(correlation_id.to_owned()),
        );
    }
    if let Some(latency) = record.latency {
        let seconds = latency.as_secs_f64();
        let millis = seconds.mul(LATENCY_MILLIS_PER_SECOND);
        let rounded = millis.mul(LATENCY_TENTHS).round().div(LATENCY_TENTHS);
        payload.insert("latency_ms".to_owned(), GuardrailValue::Decimal(rounded));
    }
    if let Some(reason) = record.reason {
        payload.insert("reason".to_owned(), GuardrailValue::Text(reason.to_owned()));
    }
    GuardrailValue::Object(payload)
}

// LiteLLM's verdict to guardrail_status pairing
const fn status_for(verdict: Verdict) -> GuardrailStatus {
    match verdict {
        Verdict::Allow => GuardrailStatus::Success,
        Verdict::Block | Verdict::Rejected => GuardrailStatus::GuardrailIntervened,
        Verdict::Throttled | Verdict::Unscanned | Verdict::Unavailable => {
            GuardrailStatus::GuardrailFailedToRespond
        }
    }
}
