use std::sync::{Mutex, PoisonError};

use betterlite_logging::exporter_options::base64_strip::Base64Stripper;
use betterlite_logging::exporter_options::failure_counter::{
    CallbackFailureCounter, handle_callback_failure,
};
use betterlite_logging::exporter_options::health::{HealthState, IntegrationHealthStatus};
use betterlite_logging::exporter_options::metadata_field::{MetadataField, select_metadata_field};
use betterlite_logging::exporter_options::options::CustomLoggerOptions;
use betterlite_logging::exporter_options::redaction::{
    ModelCallDetails, Redaction, RedactionPolicy, ResponseStamp,
};
// ast-grep-ignore: no-long-names
use betterlite_logging::exporter_options::redaction::redact_standard_logging_payload;
// ast-grep-ignore: no-long-names
use betterlite_logging::exporter_options::truncation::truncate_standard_logging_payload_content;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Options {
        options: Value,
    },
    Truncate {
        payload: Value,
    },
    MetadataField {
        request_kwargs: Option<IndexMap<String, Json>>,
    },
    Redact {
        details: DetailsInput,
        turn_off_message_logging: bool,
        #[serde(default)]
        excluded_fields: Vec<String>,
        #[serde(default)]
        redacts_messages_itself: bool,
        stamp: StampInput,
    },
    Base64 {
        payload: Value,
        max_depth: i64,
    },
    CallbackFailure {
        callback_name: String,
        counters: Vec<String>,
    },
    Health {
        statuses: Vec<StatusInput>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DetailsInput {
    standard_logging_object: Option<IndexMap<String, Json>>,
    proxy_server_request: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StampInput {
    id: String,
    created: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusInput {
    state: HealthState,
    message: Option<String>,
}

#[derive(Serialize)]
struct OptionsOutput {
    turn_off_message_logging: bool,
    message_logging: bool,
}

#[derive(Serialize)]
struct RedactOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    standard_logging_object: Option<IndexMap<String, Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy_server_request: Option<IndexMap<String, Json>>,
}

#[derive(Serialize)]
struct FailureOutput {
    calls: Vec<Vec<String>>,
}

#[derive(Serialize)]
struct HealthOutput {
    statuses: Vec<IntegrationHealthStatus>,
}

#[derive(Default)]
struct RecordingCounter {
    calls: Mutex<Vec<String>>,
}

impl RecordingCounter {
    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CallbackFailureCounter for RecordingCounter {
    // ast-grep-ignore: no-long-names
    fn increment_callback_logging_failure(&self, callback_name: &str) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(callback_name.to_owned());
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Options { options } => options_output(options),
            Input::Truncate { payload } => truncate(payload),
            Input::MetadataField { request_kwargs } => {
                encode(select_metadata_field(request_kwargs.as_ref()).map(MetadataField::as_str))
            }
            Input::Redact {
                details,
                turn_off_message_logging,
                excluded_fields,
                redacts_messages_itself,
                stamp,
            } => {
                let policy = RedactionPolicy {
                    turn_off_message_logging: *turn_off_message_logging,
                    excluded_fields: excluded_fields.clone(),
                    callback_redacts_itself: *redacts_messages_itself,
                };
                redact(details, &policy, stamp)
            }
            Input::Base64 { payload, max_depth } => base64(payload, *max_depth),
            Input::CallbackFailure {
                callback_name,
                counters,
            } => failure(callback_name, counters),
            Input::Health { statuses } => encode(HealthOutput {
                statuses: statuses.iter().map(health_status).collect(),
            }),
        }
    }
}

fn options_output(options: &Value) -> crate::helpers::HelperResult {
    let parsed: CustomLoggerOptions = decode(options)?;
    encode(OptionsOutput {
        turn_off_message_logging: parsed.turn_off_message_logging,
        message_logging: parsed.message_logging,
    })
}

fn truncate(payload: &Value) -> crate::helpers::HelperResult {
    let parsed: StandardLoggingPayload = decode(payload)?;
    // ast-grep-ignore: no-long-names
    let truncated = truncate_standard_logging_payload_content(&parsed);
    encode(truncated)
}

fn base64(payload: &Value, max_depth: i64) -> crate::helpers::HelperResult {
    let parsed: StandardLoggingPayload = decode(payload)?;
    let stripper = Base64Stripper::new().map_err(value_error)?;
    encode(stripper.strip_base64_from_messages(&parsed, max_depth))
}

fn redact(
    details: &DetailsInput,
    policy: &RedactionPolicy,
    stamp: &StampInput,
) -> crate::helpers::HelperResult {
    let call_details = ModelCallDetails {
        standard_logging_object: details.standard_logging_object.clone(),
        proxy_server_request: details.proxy_server_request.clone(),
    };
    let response_stamp = ResponseStamp {
        id: stamp.id.clone(),
        created: stamp.created,
    };
    // ast-grep-ignore: no-long-names
    let output = match redact_standard_logging_payload(&call_details, policy, &response_stamp) {
        Redaction::Unchanged => RedactOutput {
            standard_logging_object: call_details.standard_logging_object,
            proxy_server_request: call_details.proxy_server_request,
        },
        Redaction::Redacted {
            standard_logging_object,
            proxy_server_request,
        } => RedactOutput {
            standard_logging_object: Some(standard_logging_object),
            proxy_server_request,
        },
    };
    encode(output)
}

fn failure(callback_name: &str, counters: &[String]) -> crate::helpers::HelperResult {
    let recorded: Vec<RecordingCounter> = counters
        .iter()
        .map(|_| RecordingCounter::default())
        .collect();
    let mut references: Vec<&dyn CallbackFailureCounter> = Vec::with_capacity(recorded.len());
    for counter in &recorded {
        references.push(counter);
    }
    handle_callback_failure(callback_name, &references);
    encode(FailureOutput {
        calls: recorded.iter().map(RecordingCounter::calls).collect(),
    })
}

fn health_status(status: &StatusInput) -> IntegrationHealthStatus {
    match (&status.state, &status.message) {
        (HealthState::Healthy, _) => IntegrationHealthStatus::healthy(),
        (HealthState::Unhealthy, Some(message)) => {
            IntegrationHealthStatus::unhealthy(message.clone())
        }
        (HealthState::Unhealthy, None) => IntegrationHealthStatus {
            status: HealthState::Unhealthy,
            error_message: None,
        },
    }
}

fn decode<T>(value: &Value) -> Result<T, Box<crate::helpers::HelperError>>
where
    T: serde::de::DeserializeOwned,
{
    match serde_json::from_value(value.clone()) {
        Ok(decoded) => Ok(decoded),
        Err(error) => Err(value_error(error)),
    }
}

fn encode<T>(value: T) -> crate::helpers::HelperResult
where
    T: Serialize,
{
    match serde_json::to_value(value) {
        Ok(encoded) => Ok(encoded),
        Err(error) => Err(value_error(error)),
    }
}

fn value_error<E>(error: E) -> Box<crate::helpers::HelperError>
where
    E: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
