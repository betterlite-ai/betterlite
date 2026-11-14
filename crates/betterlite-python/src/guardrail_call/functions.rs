use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use pyo3::prelude::*;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_guardrails::guardrail_records::messages::messages_for_call_type as messages_for_type;
use betterlite_guardrails::guardrail_records::sanitize::sanitize_guardrail_response as sanitize_response;
use betterlite_guardrails::guardrail_records::summary::summarize_guardrail_response as summarize_response;
use betterlite_guardrails::guardrail_records::sync::sync_log_target as sync_target;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::RecordUpdate;
use super::context::{RecordStep, log_target_from_text, log_target_to_text, record_call};
use super::convert;
use super::error::CallError;

mod tests;

fn tool_cache() -> Result<DualLocalCache, CallError> {
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|source| CallError::Cache { source })?;
    let diagnostics = Diagnostics {
        redactor: Arc::new(redactor),
        emit: Arc::new(|_event| {}),
    };
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs_f64())
            .unwrap_or_default()
    });
    Ok(DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        clock,
        diagnostics,
    ))
}

#[pyfunction]
pub(crate) fn sanitize_guardrail_response(response: &str) -> PyResult<String> {
    let raw: GuardrailValue = convert::read("guardrail response", response)?;
    let cleaned = sanitize_response(convert::exact_value(raw)?).map_err(CallError::from)?;
    Ok(serde_json::to_string(&cleaned).map_err(|source| CallError::Encode { source })?)
}

#[pyfunction]
pub(crate) fn add_guardrail_record(
    data: &str,
    input: &str,
    context: &str,
) -> PyResult<RecordUpdate> {
    let record = convert::record_input::record_input_from_text(input)?;
    Ok(record_call(
        &mut None,
        RecordStep::Add(record),
        data,
        context,
    )?)
}

#[pyfunction]
pub(crate) fn sync_log_target(data: &str, target: &str) -> PyResult<String> {
    let raw: BTreeMap<String, GuardrailValue> = convert::read("request data", data)?;
    let body = convert::exact_data(raw)?;
    let mut logging = log_target_from_text(target)?;
    sync_target(&body, &mut logging);
    Ok(log_target_to_text(&logging)?)
}

#[pyfunction]
pub(crate) fn summarize_guardrail_response(
    response: &str,
    baseline: Option<&str>,
    event_type: Option<&str>,
) -> PyResult<String> {
    let raw: Option<GuardrailValue> = convert::read("guardrail response", response)?;
    let kept = raw.map(convert::exact_value).transpose()?;
    let plain: Option<BTreeMap<String, GuardrailValue>> =
        convert::read("request data", baseline.unwrap_or("null"))?;
    let original = plain.map(convert::exact_data).transpose()?;
    let hook = event_type.map(convert::event_type_from_name).transpose()?;
    let summary = summarize_response(kept.as_ref(), original.as_ref(), hook);
    Ok(serde_json::to_string(&summary).map_err(|source| CallError::Encode { source })?)
}

#[pyfunction]
pub(crate) fn messages_for_call_type(
    call_type: &str,
    data: Option<&str>,
) -> PyResult<Option<String>> {
    let raw: Option<BTreeMap<String, GuardrailValue>> =
        convert::read("request data", data.unwrap_or("null"))?;
    let body = raw.map(convert::exact_data).transpose()?;
    let cache = tool_cache()?;
    let selected = messages_for_type(call_type, body.as_ref(), &cache).map_err(CallError::from)?;
    let Some(messages) = selected else {
        return Ok(None);
    };
    Ok(Some(
        serde_json::to_string(&messages).map_err(|source| CallError::Encode { source })?,
    ))
}
