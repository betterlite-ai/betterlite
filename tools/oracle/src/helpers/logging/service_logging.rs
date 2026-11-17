use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::service_callback::level_number;
use betterlite_logging::logging_service_events::types::{ServiceLoggerPayload, ServiceType};
use betterlite_logging::logging_state::correlation::sanitize_correlation_id;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

// only the service callback section matters; the level and the queue settings come from the environment
const CONFIG_SOURCE: &str = "litellm_settings:\n  service_callback: []\n";

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    CorrelationSanitize {
        call_id: String,
    },
    LogLevel {
        environment: BTreeMap<String, String>,
    },
    ServiceSuccess {
        service: ServiceType,
        call_type: String,
        duration: f64,
        caller: Option<String>,
    },
    ServiceFailure {
        service: ServiceType,
        call_type: String,
        duration: f64,
        caller: Option<String>,
        error: String,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::CorrelationSanitize { call_id } => correlation(call_id),
            Input::LogLevel { environment } => level(environment),
            Input::ServiceSuccess { .. } | Input::ServiceFailure { .. } => payload(input),
        }
    }
}

// LiteLLM's _sanitize_correlation_id: drop the control characters, redact credentials, cap the length
fn correlation(call_id: &str) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
    Ok(json!({
        "sanitized": sanitize_correlation_id(call_id, Some(&redactor)),
    }))
}

// LiteLLM's resolve_log_level and _parse_json_logs_env, read through the service callback settings
fn level(environment: &BTreeMap<String, String>) -> HelperResult {
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(CONFIG_SOURCE)
    .map_err(failure)?;
    let settings = &config.sections.service_callback;
    Ok(json!({
        "numeric": level_number(settings.log_level),
        "json_logs": settings.json_logs,
    }))
}

// The payload LiteLLM's two async service hooks build, in the field order model_dump_json writes
fn payload(input: &Input) -> HelperResult {
    let built = match input {
        Input::ServiceSuccess {
            service,
            call_type,
            duration,
            caller,
        } => ServiceLoggerPayload {
            is_error: false,
            error: None,
            service: *service,
            duration: *duration,
            call_type: call_type.clone(),
            caller: caller.clone(),
            event_metadata: None,
        },
        Input::ServiceFailure {
            service,
            call_type,
            duration,
            caller,
            error,
        } => ServiceLoggerPayload {
            is_error: true,
            error: Some(error.clone()),
            service: *service,
            duration: *duration,
            call_type: call_type.clone(),
            caller: caller.clone(),
            event_metadata: None,
        },
        Input::CorrelationSanitize { .. } | Input::LogLevel { .. } => {
            return Err(failure("the operation records no service event"));
        }
    };
    Ok(json!({ "payloads": [built] }))
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "ServiceLoggingError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
