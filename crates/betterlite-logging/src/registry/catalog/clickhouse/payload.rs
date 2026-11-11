use serde::Serialize;

use crate::clickhouse::clickhouse_logging::payload::CallbackKwargs;
use crate::fields::payload::StandardLoggingPayload;

// LiteLLM reads the standard logging object and its request metadata out of the callback kwargs
pub(super) fn callback_kwargs(
    payload: &StandardLoggingPayload,
) -> Result<CallbackKwargs, serde_json::Error> {
    let encoded = serde_json::to_vec(&CallbackKwargsWire {
        standard_logging_object: payload,
    })?;
    serde_json::from_slice(&encoded)
}

#[derive(Serialize)]
struct CallbackKwargsWire<'payload> {
    standard_logging_object: &'payload StandardLoggingPayload,
}
