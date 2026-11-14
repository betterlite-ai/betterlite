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

// LiteLLM's _process_response override: the prompt is summarized away, the usage priced, and the provider labelled.
pub fn process_response(
    guardrail: &AzurePromptShieldGuardrail,
    context: &mut GuardrailRecordContext<'_>,
    record: ResponseRecord<'_>,
) -> Result<(), PromptShieldError> {
    let guardrail_response =
        summarize_guardrail_response(record.response, record.baseline, record.event_type);
    let tracing_detail = billing_tracing_detail(
        record.usage,
        guardrail.cost_tier,
        guardrail.price_per_1000_text_records,
    )?;
    add_guardrail_record(
        context,
        GuardrailRecordInput {
            guardrail_response,
            guardrail_status: GuardrailStatus::Success,
            start_time: record.timing.start_time,
            end_time: record.timing.end_time,
            duration: record.timing.duration,
            masked_entity_count: None,
            guardrail_provider: Some(AZURE_PROVIDER.to_owned()),
            event_type: record.event_type,
            tracing_detail,
        },
    )?;
    Ok(())
}

// LiteLLM's _process_error override: a chunk that was submitted before the block was still billed, so its usage records here.
pub fn process_error(
    guardrail: &AzurePromptShieldGuardrail,
    context: &mut GuardrailRecordContext<'_>,
    failure: &GuardrailFailure,
    record: ErrorRecord,
) -> Result<(), PromptShieldError> {
    let guardrail_status = if failure.is_intervention() {
        GuardrailStatus::GuardrailIntervened
    } else {
        GuardrailStatus::GuardrailFailedToRespond
    };
    let guardrail_response = if context.base.settings.switches.deny_text_on_error {
        GuardrailValue::Text("deny".to_owned())
    } else {
        GuardrailValue::Text(failure.to_string())
    };
    let tracing_detail = billing_tracing_detail(
        record.usage,
        guardrail.cost_tier,
        guardrail.price_per_1000_text_records,
    )?;
    add_guardrail_record(
        context,
        GuardrailRecordInput {
            guardrail_response,
            guardrail_status,
            start_time: record.timing.start_time,
            end_time: record.timing.end_time,
            duration: record.timing.duration,
            masked_entity_count: None,
            guardrail_provider: Some(AZURE_PROVIDER.to_owned()),
            event_type: record.event_type,
            tracing_detail,
        },
    )?;
    Ok(())
}
