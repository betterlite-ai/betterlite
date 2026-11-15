use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_records::record::GuardrailStatus;
use crate::presidio_actions::request_data::RequestData;
use crate::presidio_response::wire::ResponseBody;
use crate::xecguard_primitives::history::{assistant_text_from_response, build_full_history};
use crate::xecguard_primitives::redaction::sanitize_scan_result;
use crate::xecguard_primitives::transport::call_scan;
use crate::xecguard_primitives::wire::{ScanResult, ScanType, XecGuardMessage};

use super::XecGuardGuard;
use super::construction::guardrail_name;
use super::error::XecGuardRuntimeError;

const LITELLM_PARAMS_KEY: &str = "litellm_params";
const METADATA_KEY: &str = "metadata";
const GUARDRAIL_INFORMATION_KEY: &str = "standard_logging_guardrail_information";
const STANDARD_LOGGING_OBJECT_KEY: &str = "standard_logging_object";
const RESPONSE_KEY: &str = "response";

#[derive(Debug, Serialize)]
struct LoggingEntry {
    guardrail_name: String,
    guardrail_mode: GuardrailEventHooks,
    guardrail_response: BTreeMap<String, GuardrailValue>,
    guardrail_status: GuardrailStatus,
    start_time: f64,
    end_time: f64,
    duration: f64,
    masked_entity_count: Option<BTreeMap<String, i64>>,
}

pub(super) async fn scan_for_logging(
    guardrail: &XecGuardGuard,
    kwargs: &mut RequestData,
    result: &ResponseBody,
) -> Result<(), XecGuardRuntimeError> {
    if carries_guardrail_information(kwargs) {
        return Ok(());
    }
    let started = (guardrail.clock)();
    let (messages, scan_type) = history(kwargs, result)?;
    if messages.is_empty() {
        return Ok(());
    }
    let called = call_scan(
        &guardrail.client,
        &guardrail.settings,
        &messages,
        scan_type,
        true,
    )
    .await?;
    let Some(scan) = called else {
        return Ok(());
    };
    let ended = (guardrail.clock)();
    append_entry(kwargs, &logging_entry(guardrail, &scan, started, ended)?)
}

fn history(
    kwargs: &RequestData,
    result: &ResponseBody,
) -> Result<(Vec<XecGuardMessage>, ScanType), XecGuardRuntimeError> {
    let mut request_data = logged_values(kwargs)?;
    let response = response_value(result)?;
    if assistant_text_from_response(&response).is_none() {
        let messages = build_full_history(&request_data, &[], GuardrailInputType::Request);
        return Ok((messages, ScanType::Input));
    }
    request_data.insert(RESPONSE_KEY.to_owned(), response);
    let messages = build_full_history(&request_data, &[], GuardrailInputType::Response);
    Ok((messages, ScanType::Response))
}

fn logged_values(
    kwargs: &RequestData,
) -> Result<BTreeMap<String, GuardrailValue>, XecGuardRuntimeError> {
    let mut values = BTreeMap::new();
    for (key, raw) in kwargs {
        let value = serde_json::from_str(raw.get())
            .map_err(|source| XecGuardRuntimeError::LoggedJson { source })?;
        values.insert(key.clone(), value);
    }
    Ok(values)
}

fn response_value(result: &ResponseBody) -> Result<GuardrailValue, XecGuardRuntimeError> {
    let raw = result
        .clone()
        .into_raw()
        .map_err(|source| XecGuardRuntimeError::ResultBody { source })?;
    serde_json::from_str(raw.get()).map_err(|source| XecGuardRuntimeError::LoggedJson { source })
}

fn carries_guardrail_information(kwargs: &RequestData) -> bool {
    let Some(litellm_params) = kwargs.get(LITELLM_PARAMS_KEY) else {
        return false;
    };
    let Ok(litellm_params) = serde_json::from_str::<RequestData>(litellm_params.get()) else {
        return false;
    };
    let Some(metadata) = litellm_params.get(METADATA_KEY) else {
        return false;
    };
    let Ok(metadata) = serde_json::from_str::<RequestData>(metadata.get()) else {
        return false;
    };
    metadata
        .get(GUARDRAIL_INFORMATION_KEY)
        .is_some_and(|raw| carries_information(raw.get()))
}

fn carries_information(encoded: &str) -> bool {
    // only the declared list can be empty; any other value is present, so it counts as information
    match serde_json::from_str::<Option<Vec<GuardrailValue>>>(encoded) {
        Ok(Some(entries)) => !entries.is_empty(),
        Ok(None) => false,
        Err(_other_shape) => true,
    }
}

fn append_entry(
    kwargs: &mut RequestData,
    entry: &LoggingEntry,
) -> Result<(), XecGuardRuntimeError> {
    let Some(raw) = kwargs.get(STANDARD_LOGGING_OBJECT_KEY) else {
        return Err(XecGuardRuntimeError::StandardLoggingObject);
    };
    let Ok(mut standard) = serde_json::from_str::<RequestData>(raw.get()) else {
        return Err(XecGuardRuntimeError::StandardLoggingObject);
    };
    let mut entries = standard
        .get(GUARDRAIL_INFORMATION_KEY)
        .and_then(|existing| serde_json::from_str::<Vec<Box<RawValue>>>(existing.get()).ok())
        .unwrap_or_default();
    entries.push(raw_value(entry)?);
    standard.insert(GUARDRAIL_INFORMATION_KEY.to_owned(), raw_value(&entries)?);
    kwargs.insert(
        STANDARD_LOGGING_OBJECT_KEY.to_owned(),
        raw_value(&standard)?,
    );
    Ok(())
}

fn raw_value<Value>(value: &Value) -> Result<Box<RawValue>, XecGuardRuntimeError>
where
    Value: Serialize + ?Sized,
{
    let encoded = serde_json::to_string(value)
        .map_err(|source| XecGuardRuntimeError::LoggedJson { source })?;
    RawValue::from_string(encoded).map_err(|source| XecGuardRuntimeError::LoggedJson { source })
}

fn logging_entry(
    guardrail: &XecGuardGuard,
    scan: &ScanResult,
    started: SystemTime,
    ended: SystemTime,
) -> Result<LoggingEntry, XecGuardRuntimeError> {
    let start_time = epoch_seconds(started)?;
    let end_time = epoch_seconds(ended)?;
    let status = if scan.is_unsafe() {
        GuardrailStatus::GuardrailIntervened
    } else {
        GuardrailStatus::Success
    };
    Ok(LoggingEntry {
        guardrail_name: guardrail_name(&guardrail.base),
        guardrail_mode: GuardrailEventHooks::LoggingOnly,
        guardrail_response: sanitize_scan_result(scan)?,
        guardrail_status: status,
        start_time,
        end_time,
        duration: start_time.mul_add(-1.0_f64, end_time),
        masked_entity_count: None,
    })
}

fn epoch_seconds(time: SystemTime) -> Result<f64, XecGuardRuntimeError> {
    let since_epoch = time
        .duration_since(UNIX_EPOCH)
        .map_err(|source| XecGuardRuntimeError::Clock { source })?;
    Ok(since_epoch.as_secs_f64())
}
