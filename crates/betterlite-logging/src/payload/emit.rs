use std::io::{Error as IoError, Write, stdout};

use thiserror::Error;

use crate::fields::payload::StandardLoggingPayload;

pub const PRINT_PAYLOAD_ENV: &str = "LITELLM_PRINT_STANDARD_LOGGING_PAYLOAD";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EmitSettings {
    pub print_standard_logging_payload: bool,
}

impl EmitSettings {
    // os.getenv is truthy for any value but the empty string, so "0" still prints.
    #[must_use]
    pub fn from_env(raw: Option<&str>) -> Self {
        Self {
            print_standard_logging_payload: raw.is_some_and(|value| !value.is_empty()),
        }
    }
}

#[derive(Debug, Error)]
pub enum EmitError {
    #[error("the standard logging payload could not be encoded: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the standard logging payload could not be written: {source}")]
    Write {
        #[source]
        source: IoError,
    },
}

// emit_standard_logging_payload: a debug print of the payload, written where the caller sends its output.
pub fn emit_standard_logging_payload(
    payload: &StandardLoggingPayload,
    settings: &EmitSettings,
    sink: &mut dyn Write,
) -> Result<(), EmitError> {
    if !settings.print_standard_logging_payload {
        return Ok(());
    }
    let encoded = serde_json::to_vec(payload).map_err(|source| EmitError::Encode { source })?;
    sink.write_all(&encoded)
        .and_then(|()| sink.write_all(b"\n"))
        .and_then(|()| sink.flush())
        .map_err(|source| EmitError::Write { source })
}

// The lifecycle's debug print goes through an injected emitter so a test can read what it printed.
pub trait PayloadEmitter: Send + Sync {
    fn emit(&self, payload: &StandardLoggingPayload) -> Result<(), EmitError>;
}

pub struct StdoutEmitter {
    pub settings: EmitSettings,
}

impl PayloadEmitter for StdoutEmitter {
    fn emit(&self, payload: &StandardLoggingPayload) -> Result<(), EmitError> {
        let mut output = stdout().lock();
        emit_standard_logging_payload(payload, &self.settings, &mut output)
    }
}
