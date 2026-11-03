use betterlite_logging::otel::model::baggage::{TEAM_METADATA, promoted_baggage};
use betterlite_logging::otel::model::metadata::helpers::flatten_metadata;
use betterlite_logging::otel::model::metadata::llm_call_event::LLMCallEvent;
use betterlite_logging::otel::model::metadata::request_context::RequestContext;
use betterlite_logging::otel::model::metadata::request_identity::RequestIdentity;
use betterlite_logging::otel::model::metadata::request_identity::parse::from_payload;
use betterlite_logging::otel::model::metadata::wire::{
    MetadataMap, callback::Callback, payload::Payload,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Identity {
        payload: Box<Payload>,
    },
    Context {
        payload: Box<Payload>,
    },
    Session {
        kwargs: Box<Callback>,
    },
    Flatten {
        metadata: MetadataMap,
    },
    Baggage {
        identity: Box<RequestIdentity>,
        request_model: Option<String>,
        #[serde(default)]
        promoted_keys: Vec<String>,
        #[serde(default)]
        metadata_keys: Vec<String>,
        #[serde(default)]
        team_metadata_keys: Vec<String>,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Identity { payload } => Ok(json!(
                from_payload(payload).map_err(|error| invalid(&error.to_string()))?
            )),
            Input::Context { payload } => Ok(json!(
                RequestContext::from_standard_logging_payload(payload)
                    .map_err(|error| invalid(&error.to_string()))?
            )),
            Input::Session { kwargs } => Ok(
                json!({"session_id": LLMCallEvent::from_dict(kwargs).map_err(|error| invalid(&error.to_string()))?.session_id}),
            ),
            Input::Flatten { metadata } => Ok(json!(flatten_metadata(Some(metadata)))),
            Input::Baggage {
                identity,
                request_model,
                promoted_keys,
                metadata_keys,
                team_metadata_keys,
            } => normalize_baggage(promoted_baggage(
                identity,
                request_model.as_deref(),
                promoted_keys,
                metadata_keys,
                team_metadata_keys,
            )),
        }
    }
}

fn normalize_baggage(fields: BTreeMap<String, String>) -> crate::helpers::HelperResult {
    let normalized: Result<serde_json::Map<String, Value>, _> = fields
        .into_iter()
        .map(|(key, value)| {
            let item = if key == TEAM_METADATA {
                serde_json::from_str(&value)?
            } else {
                Value::String(value)
            };
            Ok((key, item))
        })
        .collect();
    normalized
        .map(Value::Object)
        .map_err(|error: serde_json::Error| invalid(&error.to_string()))
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

mod tests;
