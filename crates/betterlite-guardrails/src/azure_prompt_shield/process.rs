use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::billing::{BillingUsage, billing_tracing_detail};
use super::error::PromptShieldError;
use super::guardrail::AzurePromptShieldGuardrail;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus,
};
use crate::guardrail_records::summary::summarize_guardrail_response;

pub const AZURE_PROVIDER: &str = "azure";

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RecordTiming {
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub duration: Option<f64>,
}

pub struct ResponseRecord<'record> {
    pub response: Option<&'record GuardrailValue>,
    pub baseline: Option<&'record BTreeMap<String, GuardrailValue>>,
    pub event_type: Option<GuardrailEventHooks>,
    pub timing: RecordTiming,
    pub usage: BillingUsage,
}

pub struct ErrorRecord {
    pub event_type: Option<GuardrailEventHooks>,
    pub timing: RecordTiming,
    pub usage: BillingUsage,
}

// One invocation ended either with a response or with an error, and both paths log the same record.
pub enum ScanOutcome<'record> {
    Response(ResponseRecord<'record>),
    Error(&'record GuardrailFailure, ErrorRecord),
}

fn error_response(
    context: &GuardrailRecordContext<'_>,
    failure: &GuardrailFailure,
) -> GuardrailValue {
    if context.base.settings.switches.deny_text_on_error {
        GuardrailValue::Text("deny".to_owned())
    } else {
        GuardrailValue::Text(failure.to_string())
    }
}

const fn error_status(failure: &GuardrailFailure) -> GuardrailStatus {
    if failure.is_intervention() {
        GuardrailStatus::GuardrailIntervened
    } else {
        GuardrailStatus::GuardrailFailedToRespond
    }
}

// LiteLLM's _process_response and _process_error overrides differ only in the response and the status; both attach the priced usage and the azure provider label.
pub fn process_outcome(
    guardrail: &AzurePromptShieldGuardrail,
    context: &mut GuardrailRecordContext<'_>,
    outcome: ScanOutcome<'_>,
) -> Result<(), PromptShieldError> {
    let (guardrail_response, guardrail_status, timing, event_type, usage) = match outcome {
        ScanOutcome::Response(record) => (
            summarize_guardrail_response(record.response, record.baseline, record.event_type),
            GuardrailStatus::Success,
            record.timing,
            record.event_type,
            record.usage,
        ),
        ScanOutcome::Error(failure, record) => (
            error_response(context, failure),
            error_status(failure),
            record.timing,
            record.event_type,
            record.usage,
        ),
    };
    let tracing_detail = billing_tracing_detail(
        usage,
        guardrail.cost_tier,
        guardrail.price_per_1000_text_records,
    )?;
    add_guardrail_record(
        context,
        GuardrailRecordInput {
            guardrail_response,
            guardrail_status,
            start_time: timing.start_time,
            end_time: timing.end_time,
            duration: timing.duration,
            masked_entity_count: None,
            guardrail_provider: Some(AZURE_PROVIDER.to_owned()),
            event_type,
            tracing_detail,
        },
    )?;
    Ok(())
}
