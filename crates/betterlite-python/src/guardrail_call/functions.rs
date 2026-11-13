use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use pyo3::prelude::*;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_guardrails::guardrail_records::messages::messages_for_call_type as messages_for_type;
use betterlite_guardrails::guardrail_records::metadata::add_guardrail_record as add_record;
use betterlite_guardrails::guardrail_records::sanitize::sanitize_guardrail_response as sanitize_response;
use betterlite_guardrails::guardrail_records::summary::summarize_guardrail_response as summarize_response;
use betterlite_guardrails::guardrail_records::sync::sync_log_target as sync_target;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::context::{log_target_from_text, log_target_to_text, record_parts};
use super::convert;
use super::error::GuardrailCallError;

fn tool_cache() -> Result<DualLocalCache, GuardrailCallError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| GuardrailCallError::Cache { source })?;
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
    let value = convert::value_from_text(response)?;
    let cleaned = sanitize_response(value).map_err(GuardrailCallError::from)?;
    convert::value_to_text(&cleaned)
}

#[pyfunction]
pub(crate) fn add_guardrail_record(
    data: &str,
    record_input: &str,
    context: &str,
) -> PyResult<(String, String)> {
    let mut body = convert::data_from_text(data)?;
    let input = convert::record_input::record_input_from_text(record_input)?;
    let mut parts = record_parts(context)?;
    let mut record_context = parts.context(&mut body);
    add_record(&mut record_context, input).map_err(GuardrailCallError::from)?;
    let text = convert::data_to_text(&body)?;
    Ok((text, parts.log_target_text()?))
}

#[pyfunction]
pub(crate) fn sync_log_target(data: &str, target: &str) -> PyResult<String> {
    let body = convert::data_from_text(data)?;
    let mut logging = log_target_from_text(target)?;
    sync_target(&body, &mut logging);
    log_target_to_text(&logging)
}

#[pyfunction]
pub(crate) fn summarize_guardrail_response(
    response: &str,
    baseline: Option<&str>,
    event_type: Option<&str>,
) -> PyResult<String> {
    let parsed = convert::value_from_text(response)?;
    let kept = match parsed {
        GuardrailValue::Null => None,
        value => Some(value),
    };
    let baseline = convert::optional_data_from_text(baseline)?;
    let event_type = event_type.map(convert::event_type_from_name).transpose()?;
    let summary = summarize_response(kept.as_ref(), baseline.as_ref(), event_type);
    convert::value_to_text(&summary)
}

#[pyfunction]
pub(crate) fn messages_for_call_type(
    call_type: &str,
    data: Option<&str>,
) -> PyResult<Option<String>> {
    let body = convert::optional_data_from_text(data)?;
    let cache = tool_cache()?;
    let messages =
        messages_for_type(call_type, body.as_ref(), &cache).map_err(GuardrailCallError::from)?;
    let Some(messages) = messages else {
        return Ok(None);
    };
    let text = serde_json::to_string(&messages).map_err(|source| GuardrailCallError::Json {
        field: "guardrail messages",
        source,
    })?;
    Ok(Some(text))
}
