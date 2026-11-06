use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;
use strfmt::{FmtError, strfmt_map};
use thiserror::Error;

use super::audit::Direction;
use super::construct::PanwPrismaAirs;
use super::verdict::{Action, ScanVerdict};

// The model's own generation, which a blocked response must not hand back to the caller (LiteLLM's _CLIENT_HIDDEN_SCAN_FIELDS).
const CLIENT_HIDDEN_SCAN_FIELDS: [&str; 1] = ["response_masked_data"];
// Keys the error body sets itself, so a scan field of the same name cannot appear twice.
const BODY_KEYS: [&str; 4] = ["message", "type", "code", "guardrail"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    GuardrailViolation,
    GuardrailConfigError,
    GuardrailScanError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    PanwPrismaAirsBlocked,
    PanwPrismaAirsResponseBlocked,
    PanwPrismaAirsConfigError,
    PanwPrismaAirsScanFailed,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    #[serde(flatten)]
    pub scan_fields: IndexMap<String, Box<RawValue>>,
    pub message: String,
    #[serde(rename = "type")]
    pub kind: ErrorType,
    pub code: ErrorCode,
    pub guardrail: String,
    pub category: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub error: ErrorBody,
}

// LiteLLM raises fastapi.HTTPException(status_code, detail); the proxy turns this into the response.
#[derive(Debug, Error)]
#[error("HTTP {status}: {}", detail.error.message)]
pub struct GuardrailHttpError {
    pub status: u16,
    pub detail: Box<ErrorDetail>,
}

// Mirrors LiteLLM's fn _build_error_detail.
#[must_use]
pub fn build_error_detail(
    guardrail: &PanwPrismaAirs,
    verdict: &ScanVerdict,
    direction: Direction,
) -> ErrorDetail {
    let (action_type, code) = match direction {
        Direction::Response => ("Response", ErrorCode::PanwPrismaAirsResponseBlocked),
        Direction::Request => ("Prompt", ErrorCode::PanwPrismaAirsBlocked),
    };
    let category = verdict.category.as_deref().unwrap_or("unknown");
    let default_message =
        format!("{action_type} blocked by PANW Prisma AI Security policy (Category: {category})");
    let message = render_violation_message(
        guardrail,
        &default_message,
        [
            ("guardrail_name", guardrail.settings.guardrail_name.as_str()),
            ("category", category),
            ("action_type", action_type),
            ("default_message", default_message.as_str()),
        ],
    );
    let scan_fields = verdict
        .extra
        .iter()
        .filter(|(key, _)| {
            !key.starts_with('_')
                && !CLIENT_HIDDEN_SCAN_FIELDS.contains(&key.as_str())
                && !BODY_KEYS.contains(&key.as_str())
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    ErrorDetail {
        error: ErrorBody {
            action: Some(verdict.action),
            scan_fields,
            message,
            kind: ErrorType::GuardrailViolation,
            code,
            guardrail: guardrail.settings.guardrail_name.clone(),
            category: category.to_owned(),
        },
    }
}

// Mirrors CustomGuardrail.render_violation_message: a template that fails to format falls back to the default.
fn render_violation_message(
    guardrail: &PanwPrismaAirs,
    default: &str,
    context: [(&str, &str); 4],
) -> String {
    let Some(template) = guardrail.settings.violation_message_template.as_deref() else {
        return default.to_owned();
    };
    let rendered = strfmt_map(template, |mut field| {
        let value = context
            .iter()
            .find(|(name, _)| *name == field.key)
            .map(|(_, value)| *value)
            .ok_or_else(|| FmtError::KeyError(field.key.to_owned()))?;
        field.str(value)
    });
    rendered.unwrap_or_else(|error| {
        tracing::warn!(
            guardrail = guardrail.settings.guardrail_name,
            "Failed to format violation message template for guardrail: {error}"
        );
        default.to_owned()
    })
}
