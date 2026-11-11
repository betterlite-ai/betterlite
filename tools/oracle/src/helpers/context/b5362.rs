use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

mod cleanup;
mod filters;
mod fixture;
mod media;
mod pre_process;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Call {
    Filter(filters::FilterCall),
    Mapping(filters::MappingCall),
    PreProcess(pre_process::PreProcessCall),
    Optional(pre_process::OptionalCall),
    Cleanup(cleanup::CleanupCall),
    Media(media::MediaCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchInput {
    calls: Vec<Call>,
}

// A status becomes the call's answer, anything else means the case itself is wrong.
enum Failure {
    Status(u16),
    Invalid(String),
}

impl From<serde_json::Error> for Failure {
    fn from(error: serde_json::Error) -> Self {
        Self::Invalid(error.to_string())
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = BatchInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            let outcome = match call {
                Call::Filter(filter) => filters::run(filter),
                Call::Mapping(mapping) => filters::run_mapping(mapping),
                Call::PreProcess(pre) => pre_process::run(pre),
                Call::Optional(optional) => pre_process::run_optional(optional),
                Call::Cleanup(cleanup) => cleanup::run(cleanup),
                Call::Media(media) => media::run(media),
            };
            match outcome {
                Ok(value) => results.push(value),
                Err(Failure::Status(status)) => results.push(json!({ "error": status })),
                Err(Failure::Invalid(message)) => {
                    return Err(Box::new(HelperError {
                        class: "InvalidInput".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message,
                    }));
                }
            }
        }
        Ok(Value::Array(results))
    }
}
