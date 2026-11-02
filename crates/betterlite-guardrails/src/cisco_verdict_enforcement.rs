use std::collections::BTreeMap;
use std::ops::{Div as _, Mul as _};

use betterlite_config::sections::cisco_verdict_contracts::{FallbackAction, InspectionType};
use serde_json::value::RawValue;

use crate::cisco_configuration::records::{CiscoVerdict, Direction, ScanContext};
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_verdict::Action;
use crate::cisco_verdict_enforcement::api_error::{ApiErrorDetail, ApiErrorOutcome};
use crate::cisco_verdict_enforcement::block_payload::{
    BlockPayload, GuardrailException, error_fields_from_detail,
};
use crate::cisco_verdict_enforcement::finalize::{
    RedactionOutcome, ResponseSurface, apply_redaction_input, apply_redaction_output,
};
use crate::cisco_verdict_enforcement::logging::{REQUEST_CONTEXT_KEYS, insert_verdict_log_fields};
use crate::cisco_verdict_enforcement::support::{
    direction_label, insert_serialized, rule_name, surface_label,
};

pub mod api_error;
pub mod block_payload;
pub mod finalize;
pub mod logging;
pub mod metadata;
pub(crate) mod support;
mod tests;
pub mod verdict_resolution;

pub const REDACTED_LOG_KEYS: [&str; 5] = [
    "raw_request",
    "sanitized_payload",
    "sanitizedPayload",
    "modified_payload",
    "modifiedPayload",
];

// Mirrors LiteLLM's fn _strip_sensitive_keys and its _REDACTED_LOG_KEYS set, dropping underscore-prefixed and redacted keys recursively.
#[must_use]
pub fn strip_sensitive_keys(fields: &Fields) -> Fields {
    let mut stripped = Fields::new();
    for (key, value) in fields {
        if key.starts_with('_') || REDACTED_LOG_KEYS.contains(&key.as_str()) {
            continue;
        }
        match Json::parse(value.get()) {
            Json::Object(nested) => {
                insert_serialized(&mut stripped, key, &strip_sensitive_keys(&nested));
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                stripped.insert(key.clone(), value.clone());
            }
        }
    }
    stripped
}

// Mirrors LiteLLM's fn _build_block_payload: the audit-carrying body for a block verdict.
#[must_use]
pub fn build_block_payload(
    context: ScanContext,
    verdict: &CiscoVerdict,
    provider_name: &str,
    guardrail_name: &str,
) -> BlockPayload {
    BlockPayload {
        error: "Blocked by Cisco AI Defense Guardrail",
        message: "Blocked by Cisco AI Defense Guardrail",
        provider: provider_name.to_owned(),
        guardrail: guardrail_name.to_owned(),
        surface: surface_label(context.surface),
        direction: direction_label(context.direction),
        action: "block",
        classifications: verdict.classifications.clone(),
        severity: verdict.severity.clone(),
        rules: verdict.rules.iter().map(rule_name).collect(),
        explanation: verdict.explanation.clone(),
        event_id: verdict.event_id.clone(),
    }
}

// Mirrors LiteLLM's fn _http_exception_to_error_obj: normalizes an HTTP exception detail into a decision object.
#[must_use]
pub fn http_exception_to_error_obj(exception: &GuardrailException, guardrail_name: &str) -> Fields {
    let mut error_obj = error_fields_from_detail(&exception.detail);
    if !error_obj.contains_key("message") {
        let message = match Json::field(&error_obj, "error") {
            Json::Text(text) => text,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                "Guardrail block".to_owned()
            }
        };
        insert_serialized(&mut error_obj, "message", &message);
    }
    if !error_obj.contains_key("code") {
        insert_serialized(&mut error_obj, "code", &exception.status_code);
    }
    if !error_obj.contains_key("guardrail") {
        insert_serialized(&mut error_obj, "guardrail", guardrail_name);
    }
    error_obj
}

// Mirrors LiteLLM's fn _log_decision: one CISCO_AI_DEFENSE_DECISION log line per scan, INFO when allowed and WARN otherwise.
pub fn log_decision(
    context: ScanContext,
    verdict: &CiscoVerdict,
    duration_ms: f64,
    guardrail_name: &str,
    request_data: &Fields,
) {
    let action = verdict.action.unwrap_or(Action::Allow);
    let mut fields = Fields::new();
    insert_serialized(&mut fields, "guardrail", guardrail_name);
    insert_serialized(&mut fields, "surface", surface_label(context.surface));
    insert_serialized(&mut fields, "direction", direction_label(context.direction));
    insert_serialized(&mut fields, "action", action.as_str());
    insert_verdict_log_fields(&mut fields, verdict);
    insert_serialized(
        &mut fields,
        "duration_ms",
        &duration_ms.mul(10.0_f64).round().div(10.0_f64),
    );
    for (source_key, target_key) in REQUEST_CONTEXT_KEYS {
        if let Json::Text(value) = Json::field(request_data, source_key)
            && !value.is_empty()
        {
            insert_serialized(&mut fields, target_key, &value);
        }
    }
    let sorted: BTreeMap<&String, &Box<RawValue>> = fields.iter().collect();
    let line = format!(
        "CISCO_AI_DEFENSE_DECISION {}",
        serde_json::to_string(&sorted).unwrap_or_default()
    );
    if action == Action::Allow {
        tracing::info!("{line}");
    } else {
        tracing::warn!("{line}");
    }
}

// Mirrors LiteLLM's fn _apply_redaction; missing-action-allows and unknown-action-blocks come from cisco_verdict::normalize_action, mirroring _normalize_action.
pub fn apply_redaction(
    request_data: &mut Fields,
    response: ResponseSurface<'_>,
    context: ScanContext,
    verdict: &CiscoVerdict,
) -> RedactionOutcome {
    match context.direction {
        Direction::Input => apply_redaction_input(request_data, context.surface, verdict),
        Direction::Output => apply_redaction_output(response, context.surface, verdict),
    }
}

// Mirrors LiteLLM's fn _handle_api_error: block/monitor fallback is uniform across timeouts, connection failures and JSON-RPC error envelopes.
pub fn handle_api_error(
    surface: InspectionType,
    error_type: &str,
    fallback_on_error: FallbackAction,
) -> ApiErrorOutcome {
    tracing::error!(
        surface = surface_label(surface),
        error_type,
        "Cisco AI Defense guardrail: API communication failed"
    );
    match fallback_on_error {
        FallbackAction::Allow => {
            tracing::warn!(
                "Cisco AI Defense guardrail: API unavailable, proceeding without scanning (fallback_on_error='allow')"
            );
            ApiErrorOutcome::Unscanned
        }
        FallbackAction::Block => ApiErrorOutcome::Blocked(ApiErrorDetail {
            error: "Cisco AI Defense guardrail unavailable",
            message: "Cisco AI Defense scanning service is temporarily unavailable and fallback_on_error='block'"
                .to_owned(),
            error_type: error_type.to_owned(),
        }),
    }
}
