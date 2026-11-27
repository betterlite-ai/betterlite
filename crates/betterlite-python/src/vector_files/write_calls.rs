use std::sync::Mutex;

use betterlite_core::api::vector_file_provider::ensure_provider;
use betterlite_core::api::vector_file_write::error::VectorFileWriteError;
use betterlite_core::api::vector_file_write::error::failure as write_failure;
use betterlite_core::api::vector_file_write::{VectorFileWriteDeps, VectorFileWrites};
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
impl SessionCore {
    pub(crate) async fn write_call(
        &self,
        wire: &WriteWire,
        side: WriteSide,
        asynchronous: bool,
    ) -> Result<Json, CallError> {
        let provider = ensure_provider(wire.custom_llm_provider.as_deref()).to_owned();
        let logging =
            Mutex::new(
                self.logging_state(side.call_type())
                    .map_err(|source| CallError {
                        provider: provider.clone(),
                        source: CallFailure::Write(Box::new(write_failure(
                            VectorFileWriteError::Logging(source),
                            Some(&provider),
                            &self.redactor,
                        ))),
                    })?,
            );
        let deps = VectorFileWriteDeps {
            logging: &logging,
            registry: None,
            redactor: &self.redactor,
            request_timeout_seconds: self.request_timeout_seconds,
        };
        let writes = VectorFileWrites {
            service: self,
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
}
