use std::sync::Mutex;

use betterlite_core::api::vector_file_provider::ensure_provider;
use betterlite_core::api::vector_file_write::error::VectorFileWriteError;
use betterlite_core::api::vector_file_write::error::failure as write_failure;
use betterlite_core::api::vector_file_write::{VectorFileWriteDeps, VectorFileWrites};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;

use super::convert::write::{update_request, write_request};
use super::convert::{CallError, CallFailure, WriteWire};
use super::session::SessionCore;

// Which write a wire stands for: update carries a body, delete does not
#[derive(Clone, Copy)]
pub(crate) enum WriteSide {
    Update,
    Delete,
}

impl WriteSide {
    pub(crate) const fn call_type(self) -> &'static str {
        match self {
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

// B1074's update and delete, in their own module so the create/list driver stays a separate unit
pub(crate) async fn write_call(
    core: &SessionCore,
    wire: &WriteWire,
    side: WriteSide,
    asynchronous: bool,
) -> Result<Json, CallError> {
    let provider = ensure_provider(wire.custom_llm_provider.as_deref()).to_owned();
    let logging = write_logging(core, side, &provider)?;
    let deps = VectorFileWriteDeps {
        logging: &logging,
        registry: None,
        redactor: &core.redactor,
        request_timeout_seconds: core.request_timeout_seconds,
    };
    let writes = VectorFileWrites {
        service: core,
        deps,
    };
    let answer = match side {
        WriteSide::Update => {
            let request = update_request(wire);
            if asynchronous {
                writes.aupdate(&request).await
            } else {
                writes.update(&request).await
            }
        }
        WriteSide::Delete => {
            let request = write_request(wire);
            if asynchronous {
                writes.adelete(&request).await
            } else {
                writes.delete(&request).await
            }
        }
    };
    answer.map_err(|source| CallError {
        provider,
        source: CallFailure::Write(Box::new(source)),
    })
}

// The update or delete pre-call state, with the family error a logging failure carries
fn write_logging(
    core: &SessionCore,
    side: WriteSide,
    provider: &str,
) -> Result<Mutex<LoggingState>, CallError> {
    let state = core
        .logging_state(side.call_type())
        .map_err(|source| CallError {
            provider: provider.to_owned(),
            source: CallFailure::Write(Box::new(write_failure(
                VectorFileWriteError::Logging(source),
                Some(provider),
                &core.redactor,
            ))),
        })?;
    Ok(Mutex::new(state))
}
