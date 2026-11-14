use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_records::record::GuardrailStatus;

use super::error::{NomaBlockedMessage, NomaError};
use super::request::as_object;
use super::wire::ScanResponse;
use super::{MessageRole, NomaSettings};

pub(super) const SENSITIVE_DATA_DETECTOR_KEYS: [&str; 2] = ["sensitiveData", "dataDetector"];

pub(super) enum Verdict {
    Allowed,
    Anonymized(String),
}

// LiteLLM reads aggregatedScanResult=True as unsafe; a missing key defaults to False (safe)
pub(super) const fn determine_guardrail_status(response: &ScanResponse) -> GuardrailStatus {
    match &response.aggregated_scan_result {
        None | Some(Some(GuardrailValue::Boolean(false))) => GuardrailStatus::Success,
        Some(Some(GuardrailValue::Boolean(true))) => GuardrailStatus::GuardrailIntervened,
        Some(None | Some(_)) => GuardrailStatus::GuardrailFailedToRespond,
    }
}

// Sensitive data is the only detector whose failures anonymization may resolve
pub(super) fn only_sensitive_data_failed(
    classification: &BTreeMap<String, GuardrailValue>,
) -> bool {
    let mut sensitive_detected = false;
    let mut other_detected = false;
    for (key, value) in classification {
        let Some(fields) = as_object(value) else {
            continue;
        };
        let sensitive = SENSITIVE_DATA_DETECTOR_KEYS.contains(&key.as_str());
        let detected = if !sensitive && fields.contains_key("result") {
            is_result_true(value)
        } else {
            fields.values().any(is_result_true)
        };
        if sensitive {
            sensitive_detected |= detected;
        } else {
            other_detected |= detected;
        }
    }
    sensitive_detected && !other_detected
}

pub(super) fn extract_anonymized_content(
    response: &ScanResponse,
    role: MessageRole,
) -> Result<Option<String>, NomaError> {
    let Some(Some(scan_result)) = response.scan_result.as_ref() else {
        return Ok(None);
    };
    let GuardrailValue::Array(items) = scan_result else {
        return Err(NomaError::ResponseField {
            field: "scanResult",
        });
    };
    for item in items {
        let Some(fields) = as_object(item) else {
            return Err(NomaError::ResponseField {
                field: "scanResult",
            });
        };
        if has_role(fields, role) {
            return anonymized_in(fields);
        }
    }
    Ok(None)
}

// LiteLLM skips anonymization in monitor mode, when the flag is off, and when another detector failed
pub(super) fn should_anonymize(
    settings: &NomaSettings,
    response: &ScanResponse,
    role: MessageRole,
) -> Result<bool, NomaError> {
    if settings.monitor_mode || !settings.anonymize_input {
        return Ok(false);
    }
    let safe = matches!(
        &response.aggregated_scan_result,
        None | Some(None | Some(GuardrailValue::Boolean(false)))
    );
    if safe {
        return Ok(true);
    }
    let Some(Some(GuardrailValue::Array(items))) = response.scan_result.as_ref() else {
        return Ok(false);
    };
    for item in items {
        let Some(fields) = as_object(item) else {
            return Err(NomaError::ResponseField {
                field: "scanResult",
            });
        };
        if !has_role(fields, role) {
            continue;
        }
        let results = match fields.get("results") {
            None | Some(GuardrailValue::Null) => return Ok(false),
            Some(found) => as_object(found).ok_or(NomaError::ResponseField {
                field: "scanResult.results",
            })?,
        };
        return Ok(only_sensitive_data_failed(results));
    }
    Ok(false)
}

pub(super) fn check_verdict(
    settings: &NomaSettings,
    role: MessageRole,
    message: &str,
    response: &ScanResponse,
) -> Result<(), NomaError> {
    let blocked = matches!(
        &response.aggregated_scan_result,
        Some(Some(GuardrailValue::Boolean(true)))
    );
    if !blocked {
        let text = format!(
            "Noma guardrail allowed {} message: {message}",
            role.as_str()
        );
        if settings.monitor_mode {
            tracing::info!("{text}");
        } else {
            tracing::debug!("{text}");
        }
        return Ok(());
    }
    let text = format!(
        "Noma guardrail blocked {} message: {message}",
        role.as_str()
    );
    if settings.monitor_mode {
        tracing::warn!("{text}");
        return Ok(());
    }
    tracing::debug!("{text}");
    let details = match response.scan_result.as_ref() {
        None => GuardrailValue::Object(BTreeMap::new()),
        Some(None) => GuardrailValue::Null,
        Some(Some(found)) => found.clone(),
    };
    Err(NomaError::Blocked(NomaBlockedMessage { details }))
}

pub(super) fn handle_verdict_background(role: MessageRole, message: &str, response: &ScanResponse) {
    let blocked = matches!(
        &response.aggregated_scan_result,
        Some(Some(GuardrailValue::Boolean(true)))
    );
    if blocked {
        tracing::warn!(
            "Noma guardrail blocked {} message: {message}",
            role.as_str()
        );
    } else {
        tracing::info!(
            "Noma guardrail allowed {} message: {message}",
            role.as_str()
        );
    }
}

pub(super) fn decide_verdict(
    settings: &NomaSettings,
    role: MessageRole,
    message: &str,
    response: &ScanResponse,
) -> Result<Verdict, NomaError> {
    if settings.monitor_mode {
        handle_verdict_background(role, message, response);
        return Ok(Verdict::Allowed);
    }
    if should_anonymize(settings, response, role)?
        && let Some(anonymized) = extract_anonymized_content(response, role)?
    {
        return Ok(Verdict::Anonymized(anonymized));
    }
    check_verdict(settings, role, message, response)?;
    Ok(Verdict::Allowed)
}

fn has_role(fields: &BTreeMap<String, GuardrailValue>, role: MessageRole) -> bool {
    matches!(fields.get("role"), Some(GuardrailValue::Text(found)) if found == role.as_str())
}

fn is_result_true(value: &GuardrailValue) -> bool {
    let Some(fields) = as_object(value) else {
        return false;
    };
    matches!(fields.get("result"), Some(GuardrailValue::Boolean(true)))
}

fn anonymized_in(fields: &BTreeMap<String, GuardrailValue>) -> Result<Option<String>, NomaError> {
    let Some(results_value) = fields.get("results") else {
        return Ok(None);
    };
    let Some(results) = as_object(results_value) else {
        return Err(NomaError::ResponseField {
            field: "scanResult.results",
        });
    };
    let Some(anonymized_value) = results.get("anonymizedContent") else {
        return Ok(None);
    };
    let Some(anonymized_content) = as_object(anonymized_value) else {
        return Err(NomaError::ResponseField {
            field: "anonymizedContent",
        });
    };
    match anonymized_content.get("anonymized") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok((!text.is_empty()).then_some(text.clone())),
        Some(_) => Err(NomaError::ResponseField {
            field: "anonymizedContent.anonymized",
        }),
    }
}
