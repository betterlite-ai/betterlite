use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use crate::clickhouse::clickhouse_logging::payload::CallbackKwargs;
use crate::fields::payload::StandardLoggingPayload;

// LiteLLM reads the standard logging object and the request metadata out of the callback kwargs, and the session id falls back to that metadata
pub(super) fn callback_kwargs(
    payload: &StandardLoggingPayload,
) -> Result<CallbackKwargs, serde_json::Error> {
    let request_metadata = match payload.metadata.requester_metadata.as_ref() {
        Some(Json::Object(fields)) => Some(fields),
        _ => None,
    };
    let encoded = serde_json::to_vec(&CallbackKwargsWire {
        standard_logging_object: payload,
        litellm_params: LitellmParamsWire {
            metadata: request_metadata,
        },
    })?;
    serde_json::from_slice(&encoded)
}

#[derive(Serialize)]
struct CallbackKwargsWire<'payload> {
    standard_logging_object: &'payload StandardLoggingPayload,
    litellm_params: LitellmParamsWire<'payload>,
}

#[derive(Serialize)]
struct LitellmParamsWire<'payload> {
    metadata: Option<&'payload IndexMap<String, Json>>,
}
