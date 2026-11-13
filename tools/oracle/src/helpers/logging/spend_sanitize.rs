mod spec;
mod tests;

use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_logging::spend_sanitize::errors::sanitize_error_information;
use betterlite_logging::spend_sanitize::guardrails::sanitize_guardrail_information;
use betterlite_logging::spend_sanitize::projection::convert_to_json_serializable;
use betterlite_logging::spend_sanitize::raw_model::scrub_raw_model_from_errors;
use betterlite_logging::spend_sanitize::request_body::sanitize_request_body;
use betterlite_logging::spend_sanitize::value::{SpendMapping, SpendValue};
use serde::Serialize;
use serde_json::{Value, json};
use spec::Decoder;

#[derive(serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SanitizeRequestBody {
        bodies: Vec<BodySpec>,
    },
    SanitizeErrorInformation {
        information: Value,
        store_bodies: bool,
        max_string_length: i64,
    },
    SanitizeGuardrailInformation {
        information: Value,
        store_bodies: bool,
    },
    ScrubRawModel {
        information: Value,
        raw_model: String,
    },
    SanitizeAndProject {
        sanitize_body: Value,
        project_body: Value,
        max_string_length: i64,
    },
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodySpec {
    body: Value,
    max_string_length: i64,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::SanitizeRequestBody { bodies } => sanitize_bodies(bodies),
            Input::SanitizeErrorInformation {
                information,
                store_bodies,
                max_string_length,
            } => sanitize_error(information, *store_bodies, *max_string_length),
            Input::SanitizeGuardrailInformation {
                information,
                store_bodies,
            } => {
                let decoded = decode(information)?;
                let policy = SpendSanitizePolicy {
                    store_bodies: *store_bodies,
                    max_string_length: 2048,
                };
                let sanitized = sanitize_guardrail_information(&decoded, &policy);
                Ok(json!({ "result": to_value(sanitized)? }))
            }
            Input::ScrubRawModel {
                information,
                raw_model,
            } => scrub_model(information, raw_model),
            Input::SanitizeAndProject {
                sanitize_body,
                project_body,
                max_string_length,
            } => sanitize_and_project(sanitize_body, project_body, *max_string_length),
        }
    }
}

fn sanitize_bodies(bodies: &[BodySpec]) -> crate::helpers::HelperResult {
    let sanitized = bodies
        .iter()
        .map(|spec| {
            let body = decode(&spec.body)?;
            let clean = sanitize_request_body(mapping_of(&body)?, spec.max_string_length);
            to_value(clean)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "result": sanitized }))
}

fn sanitize_error(
    information: &Value,
    store_bodies: bool,
    max_string_length: i64,
) -> crate::helpers::HelperResult {
    let decoded = decode(information)?;
    let policy = SpendSanitizePolicy {
        store_bodies,
        max_string_length,
    };
    let sanitized = sanitize_error_information(mapping_of(&decoded)?, &policy, None);
    Ok(json!({ "result": to_value(sanitized)? }))
}

fn scrub_model(information: &Value, raw_model: &str) -> crate::helpers::HelperResult {
    let decoded = decode(information)?;
    let scrubbed = scrub_raw_model_from_errors(mapping_of(&decoded)?, raw_model);
    Ok(json!({ "result": to_value(scrubbed)? }))
}

fn sanitize_and_project(
    sanitize_body: &Value,
    project_body: &Value,
    max_string_length: i64,
) -> crate::helpers::HelperResult {
    let body = decode(sanitize_body)?;
    let sanitized = sanitize_request_body(mapping_of(&body)?, max_string_length);
    let projected = decode(project_body)?;
    Ok(json!({
        "result": {
            "sanitized": to_value(sanitized)?,
            "projected": to_value(convert_to_json_serializable(&projected))?,
        },
    }))
}

fn decode(value: &Value) -> Result<SpendValue, Box<crate::helpers::HelperError>> {
    match Decoder::default().decode(value) {
        Ok(decoded) => Ok(decoded),
        Err(message) => Err(invalid(&message)),
    }
}

fn mapping_of(value: &SpendValue) -> Result<&SpendMapping, Box<crate::helpers::HelperError>> {
    let SpendValue::Mapping(mapping) = value else {
        return Err(invalid("the sanitizer input must be a mapping"));
    };
    Ok(mapping)
}

fn to_value<T>(value: T) -> Result<Value, Box<crate::helpers::HelperError>>
where
    T: Serialize,
{
    match serde_json::to_value(value) {
        Ok(converted) => Ok(converted),
        Err(error) => Err(invalid(&error.to_string())),
    }
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
