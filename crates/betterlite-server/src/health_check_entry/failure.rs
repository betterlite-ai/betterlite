use std::error::Error as StdError;
use std::sync::PoisonError;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{HealthCheckEntry, HealthCheckFailure, HealthEntryError, RawRequestRecord};
use crate::health_probe_modes::logging::HealthProbeLogging;

pub(super) const MAXIMUM_STACK_LENGTH: usize = 1000;
const RAW_REQUEST_KEY: &str = "raw_request_typed_dict";
const STACK_SEPARATOR: &str = "\nstack trace: ";
// LiteLLM's missing-mode text puts two spaces before the newline and spells the marker as one word
const MISSING_MODE_SUFFIX: &str = ". Missing `mode`. Set the `mode` for the model - https://docs.litellm.ai/docs/proxy/health#embedding-models  \nstacktrace: ";

impl HealthCheckEntry {
    pub(super) fn failure(
        &self,
        mode: Option<&str>,
        error: HealthEntryError,
        logging: Option<&dyn HealthProbeLogging>,
    ) -> HealthCheckFailure {
        let stack = self.stack_text(&error);
        let message = error.to_string();
        match mode {
            None => HealthCheckFailure {
                error: format!("error:{message}{MISSING_MODE_SUFFIX}{stack}"),
                raw_request: None,
                exception: error,
            },
            Some(_) => HealthCheckFailure {
                error: format!("{message}{STACK_SEPARATOR}{stack}"),
                raw_request: raw_request(logging),
                exception: error,
            },
        }
    }

    // betterlite has no traceback: the stack text is the error and its source chain, one cause per line
    fn stack_text(&self, error: &HealthEntryError) -> String {
        let mut text = error.to_string();
        let mut cause = StdError::source(error);
        while let Some(source) = cause {
            text.push('\n');
            text.push_str(&source.to_string());
            cause = source.source();
        }
        self.cut_stack(text)
    }

    fn cut_stack(&self, text: String) -> String {
        let scrubbed = if self.redact_secrets {
            self.redactor.redact_string(&text)
        } else {
            text
        };
        scrubbed.chars().take(MAXIMUM_STACK_LENGTH).collect()
    }
}

fn raw_request(logging: Option<&dyn HealthProbeLogging>) -> Option<RawRequestRecord> {
    let state = logging?.logging_state()?;
    let details = state.lock().unwrap_or_else(PoisonError::into_inner);
    record_from(details.model_call_details.get(RAW_REQUEST_KEY)?)
}

fn record_from(value: &Json) -> Option<RawRequestRecord> {
    let Json::Object(fields) = value else {
        return None;
    };
    Some(RawRequestRecord {
        raw_request_api_base: text_field(fields, "raw_request_api_base"),
        raw_request_body: json_field(fields, "raw_request_body"),
        raw_request_headers: json_field(fields, "raw_request_headers"),
        error: text_field(fields, "error"),
    })
}

fn text_field(fields: &IndexMap<String, Json>, key: &str) -> Option<String> {
    match fields.get(key) {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn json_field(fields: &IndexMap<String, Json>, key: &str) -> Option<Json> {
    match fields.get(key) {
        None | Some(Json::Null) => None,
        Some(field) => Some(field.clone()),
    }
}
