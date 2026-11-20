use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::error::BedrockDecisionError;
use super::exceptions::{merged_response_status, response_status};
use super::tracing::{Pricing, tracing_detail};
use super::value::json_to_value;
use crate::bedrock_records::{Response, Usage};
use crate::guardrail_records::metadata::add_guardrail_record;
use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus, GuardrailTracingDetail,
};

pub struct CallLog<'call> {
    pub guardrail_provider: &'call str,
    pub event_type: Option<GuardrailEventHooks>,
    pub start_time: SystemTime,
    pub now: SystemTime,
    pub pricing: Pricing<'call>,
}

pub struct AttemptLog<'attempt> {
    pub status_code: u16,
    pub body: &'attempt [u8],
    pub json_response: &'attempt Response,
    pub completed_chunk_usages: &'attempt [Usage],
}

// LiteLLM's _log_apply_guardrail_attempt: one record whose usage spans every billed call so far
pub fn log_attempt(
    context: &mut GuardrailRecordContext<'_>,
    call: &CallLog<'_>,
    attempt: &AttemptLog<'_>,
) -> Result<(), BedrockDecisionError> {
    let mut logged = attempt.json_response.clone();
    if !attempt.completed_chunk_usages.is_empty() {
        let mut usages: Vec<&Usage> = attempt.completed_chunk_usages.iter().collect();
        if let Some(blocking) = attempt.json_response.usage.as_ref() {
            usages.push(blocking);
        }
        let summed = Response {
            usage: Some(sum_usages(&usages)?),
            action: None,
            output: None,
            outputs: None,
            assessments: None,
            extra: BTreeMap::new(),
        };
        // an explicit null usage stays in extra, so the summed one replaces it there too
        logged.absorb(&summed);
    }
    let detail = tracing_detail(&logged, &call.pricing)?;
    let status = response_status(attempt.status_code, attempt.body);
    record(context, call, &json_of(&logged)?, status, Some(detail))
}

// LiteLLM's _log_apply_guardrail_success: the merged response carries the status, marker included
pub fn log_success(
    context: &mut GuardrailRecordContext<'_>,
    call: &CallLog<'_>,
    merged: &Response,
) -> Result<(), BedrockDecisionError> {
    let detail = tracing_detail(merged, &call.pricing)?;
    let status = merged_response_status(merged);
    record(context, call, &json_of(merged)?, status, Some(detail))
}

// LiteLLM's _log_apply_guardrail_failure: the chunks AWS billed before the failure still carry cost
pub fn log_failure(
    context: &mut GuardrailRecordContext<'_>,
    call: &CallLog<'_>,
    detail: &str,
    completed_chunk_usages: &[Usage],
) -> Result<(), BedrockDecisionError> {
    let mut fields = IndexMap::from([("error".to_owned(), Json::Text(detail.to_owned()))]);
    let tracing = if completed_chunk_usages.is_empty() {
        None
    } else {
        let usages: Vec<&Usage> = completed_chunk_usages.iter().collect();
        let billed = sum_usages(&usages)?;
        fields.insert("usage".to_owned(), json_of(&billed)?);
        let response = Response {
            usage: Some(billed),
            action: None,
            output: None,
            outputs: None,
            assessments: None,
            extra: BTreeMap::new(),
        };
        Some(tracing_detail(&response, &call.pricing)?)
    };
    let status = GuardrailStatus::GuardrailFailedToRespond;
    record(context, call, &Json::Object(fields), status, tracing)
}

fn record(
    context: &mut GuardrailRecordContext<'_>,
    call: &CallLog<'_>,
    response: &Json,
    status: GuardrailStatus,
    detail: Option<GuardrailTracingDetail>,
) -> Result<(), BedrockDecisionError> {
    let started = call
        .start_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let ended = call
        .now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    // float_arithmetic is denied; the elapsed duration comes from the clock directly
    let duration = call
        .now
        .duration_since(call.start_time)
        .unwrap_or_default()
        .as_secs_f64();
    add_guardrail_record(
        context,
        GuardrailRecordInput {
            guardrail_response: json_to_value(response),
            guardrail_status: status,
            start_time: Some(started),
            end_time: Some(ended),
            duration: Some(duration),
            masked_entity_count: None,
            guardrail_provider: Some(call.guardrail_provider.to_owned()),
            event_type: call.event_type,
            tracing_detail: detail.filter(|value| *value != GuardrailTracingDetail::default()),
        },
    )?;
    Ok(())
}

// LiteLLM's _sum_usage_counters: the union of counters, with an absent one counting zero
fn sum_usages(usages: &[&Usage]) -> Result<Usage, BedrockDecisionError> {
    let mut totals: BTreeMap<String, Option<i64>> = BTreeMap::new();
    for usage in usages {
        for (counter, count) in &usage.0 {
            let total = totals.entry(counter.clone()).or_insert(Some(0));
            let sum = total
                .unwrap_or_default()
                .checked_add(count.unwrap_or_default());
            *total = Some(sum.ok_or_else(|| BedrockDecisionError::UsageOverflow {
                counter: counter.clone(),
            })?);
        }
    }
    Ok(Usage(totals))
}

fn json_of<T>(value: &T) -> Result<Json, BedrockDecisionError>
where
    T: Serialize,
{
    serde_json::to_value(value)
        .and_then(serde_json::from_value::<Json>)
        .map_err(|source| BedrockDecisionError::Encode { source })
}
