use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::header::HeaderMap;
use secrecy::SecretString;

use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::hiddenlayer::evaluation::error::EvaluationError;
use crate::hiddenlayer::hiddenlayer_authentication::error::HiddenlayerError;
use crate::hiddenlayer::hiddenlayer_authentication::settings::{
    DEFAULT_AUTH_TIMEOUT, HiddenlayerCredentials,
};

pub mod error;
pub mod metadata;
pub mod wire;

#[cfg(test)]
pub(crate) mod tests;

pub const BLOCK_MESSAGE: &str = "Blocked by Hiddenlayer.";
pub const VIOLATED_POLICY: &str = "Violated guardrail policy";
pub const PROVIDER: &str = "litellm";
pub const ACTION_BLOCK: &str = "Block";
pub const ACTION_REDACT: &str = "Redact";

// The parameters initialize_guardrail hands to either version's constructor
#[derive(Debug, Clone)]
pub struct EvaluationArgs {
    pub guardrail_name: Option<String>,
    pub event_hook: LitellmParamsMode,
    pub api_id: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub auth_url: Option<String>,
    pub timeout: Option<f64>,
    pub default_on: GuardrailFlag,
}

impl EvaluationArgs {
    #[must_use]
    pub fn credentials(&self) -> HiddenlayerCredentials {
        HiddenlayerCredentials {
            api_id: self.api_id.clone(),
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            auth_url: self.auth_url.clone(),
            timeout: self.timeout,
        }
    }
}

#[must_use]
pub fn guardrail_settings(
    args: &EvaluationArgs,
    supported: Option<Vec<GuardrailEventHooks>>,
) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: args.guardrail_name.clone(),
        supported_event_hooks: supported,
        event_hook: Some(args.event_hook.clone()),
        default_on: args.default_on,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: GuardrailSwitches {
            use_native_during_call_hook: false,
            use_native_lifecycle_hooks: false,
            records_own_information: false,
            uses_apply_guardrail_interface: true,
            deny_text_on_error: false,
        },
    }
}

pub fn evaluation_timeout(seconds: Option<f64>) -> Result<Duration, HiddenlayerError> {
    match seconds {
        Some(seconds) => Duration::try_from_secs_f64(seconds)
            .map_err(|source| HiddenlayerError::InvalidTimeout { source }),
        None => Ok(DEFAULT_AUTH_TIMEOUT),
    }
}

#[must_use]
pub fn config_model() -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "model".to_owned(),
            GuardrailValue::Text("HiddenlayerGuardrailConfigModel".to_owned()),
        ),
        (
            "ui_friendly_name".to_owned(),
            GuardrailValue::Text("Hiddenlayer Guardrail".to_owned()),
        ),
    ]))
}

// Both versions name the same provider and differ in their version and their own hl- headers
pub fn detection_headers(
    version: &str,
    project_id: Option<&str>,
    hl_headers: Option<&BTreeMap<String, String>>,
) -> Result<HeaderMap, EvaluationError> {
    let mut pairs = vec![
        ("content-type", "application/json"),
        ("hl-runtime-edge-provider", PROVIDER),
        ("hl-runtime-edge-provider-version", version),
    ];
    if let Some(project_id) = project_id {
        pairs.push((metadata::PROJECT_HEADER, project_id));
    }
    for (name, value) in hl_headers.into_iter().flatten() {
        pairs.push((name.as_str(), value.as_str()));
    }
    wire::header_map(&pairs)
}

// The only content parts both redaction paths read, a text part's text
#[must_use]
pub fn text_part(part: &GuardrailValue) -> Option<String> {
    let GuardrailValue::Object(fields) = part else {
        return None;
    };
    let (Some(GuardrailValue::Text(kind)), Some(GuardrailValue::Text(text))) =
        (fields.get("type"), fields.get("text"))
    else {
        return None;
    };
    (kind == "text").then(|| text.clone())
}

// A redacted message keeps every field the endpoint returned; a role it left out reads as user
pub fn as_guardrail_message(message: &GuardrailValue) -> Result<GuardrailMessage, EvaluationError> {
    let GuardrailValue::Object(fields) = message else {
        return Err(EvaluationError::Shape { field: "messages" });
    };
    let mut extra = fields.clone();
    let role = match extra.remove("role") {
        Some(GuardrailValue::Text(role)) => role,
        _ => "user".to_owned(),
    };
    Ok(GuardrailMessage { role, extra })
}
