use std::sync::Mutex;

use betterlite_core::api::vector_file_create::error::FileCallError;
use betterlite_core::api::vector_file_create::error::failure as file_failure;
use betterlite_core::api::vector_file_create::{VectorFileCreateDeps, VectorFileCreates};
use betterlite_core::api::vector_file_http_read::service::LogContext;
use betterlite_core::api::vector_file_provider::ensure_provider;
use betterlite_core::api::vector_file_read::error::VectorFileReadError;
use betterlite_core::api::vector_file_read::service::{ReadSession, SdkDeps};
use betterlite_core::api::vector_file_write::error::VectorFileWriteError;
use betterlite_core::api::vector_file_write::error::failure as write_failure;
use betterlite_core::api::vector_file_write::{VectorFileWriteDeps, VectorFileWrites};
use betterlite_models::decisions::Json;

use super::convert::{
    CallError, CallFailure, CreateWire, ListWire, ReadWire, WriteWire, create_request,
    list_request, read_call, update_request, write_request,
};
use super::session::{Pending, PendingOperation, SessionCore};
use super::source::SilentSink;

// Which read a Python call stands for: the file object, or its parsed content
#[derive(Clone, Copy)]
enum ReadSide {
    Retrieve,
    Content,
}

impl ReadSide {
    const fn call_type(self) -> &'static str {
        match self {
            Self::Retrieve => "retrieve",
            Self::Content => "retrieve_content",
        }
    }
}

// Everything one native call runs, with the provider its failure names and the family error the service gave
impl SessionCore {
    pub(crate) async fn run(&self, pending: Pending) -> Result<Json, CallError> {
        let asynchronous = pending.asynchronous;
        match pending.operation {
            PendingOperation::Create(wire) => self.create(&wire, asynchronous).await,
            PendingOperation::List(wire) => self.list(&wire, asynchronous).await,
            PendingOperation::Retrieve(wire) => {
                self.read(&wire, ReadSide::Retrieve, asynchronous).await
            }
            PendingOperation::RetrieveContent(wire) => {
                self.read(&wire, ReadSide::Content, asynchronous).await
            }
            PendingOperation::Update(wire) => self.update(&wire, asynchronous).await,
            PendingOperation::Delete(wire) => self.delete(&wire, asynchronous).await,
        }
    }

    // B1072's create: the flag picks the async handler, and the service answers with the file object
    async fn create(&self, wire: &CreateWire, asynchronous: bool) -> Result<Json, CallError> {
        let provider = ensure_provider(wire.custom_llm_provider.as_deref()).to_owned();
        let logging = Mutex::new(self.logging_state("create").map_err(|source| CallError {
            provider: provider.clone(),
            source: CallFailure::File(file_failure(
                &FileCallError::Logging(source),
                &provider,
                &self.redactor,
            )),
        })?);
        let deps = VectorFileCreateDeps {
            logging: &logging,
            redactor: &self.redactor,
            registry: None,
            request_timeout_seconds: self.request_timeout_seconds,
        };
        let creates = VectorFileCreates {
            service: self,
            deps,
        };
        let request = create_request(wire);
        let answer = if asynchronous {
            creates.acreate(&request).await
        } else {
            creates.create(&request).await
        };
        answer.map_err(|source| CallError {
            provider,
            source: CallFailure::File(source),
        })
    }

    // B1072's list: the five query fields are the caller's, and the answer is the file list page
    async fn list(&self, wire: &ListWire, asynchronous: bool) -> Result<Json, CallError> {
        let provider = ensure_provider(wire.custom_llm_provider.as_deref()).to_owned();
        let logging = Mutex::new(self.logging_state("list").map_err(|source| CallError {
            provider: provider.clone(),
            source: CallFailure::File(file_failure(
                &FileCallError::Logging(source),
                &provider,
                &self.redactor,
            )),
        })?);
        let deps = VectorFileCreateDeps {
            logging: &logging,
            redactor: &self.redactor,
            registry: None,
            request_timeout_seconds: self.request_timeout_seconds,
        };
        let creates = VectorFileCreates {
            service: self,
            deps,
        };
        let request = list_request(wire);
        let answer = if asynchronous {
            creates.alist(&request).await
        } else {
            creates.list(&request).await
        };
        answer.map_err(|source| CallError {
            provider,
            source: CallFailure::File(source),
        })
    }

    // B1073's retrieve or content retrieve, with the read session holding the configs and the client
    async fn read(
        &self,
        wire: &ReadWire,
        side: ReadSide,
        asynchronous: bool,
    ) -> Result<Json, CallError> {
        let provider = ensure_provider(wire.custom_llm_provider.as_deref()).to_owned();
        let mut state = self
            .logging_state(side.call_type())
            .map_err(|source| CallError {
                provider: provider.clone(),
                source: CallFailure::Read(VectorFileReadError::Logging(source)),
            })?;
        let call = read_call(wire);
        let deps = SdkDeps {
            configs: &self.configs,
            registry: None,
            source: &self.source,
            supplied: None,
            request_timeout_seconds: self.request_timeout_seconds,
        };
        let sink = SilentSink;
        let env = self.pre_post(&sink);
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        let mut session = ReadSession {
            deps: &deps,
            call: &call,
            log: &mut log,
        };
        let answer = match side {
            ReadSide::Retrieve if asynchronous => session.aretrieve().await,
            ReadSide::Retrieve => session.retrieve().await,
            ReadSide::Content if asynchronous => session.aretrieve_content().await,
            ReadSide::Content => session.retrieve_content().await,
        };
        answer.map_err(|source| CallError {
            provider,
            source: CallFailure::Read(source),
        })
    }

    // B1074's update: attributes and the body are the caller's, and an empty dict is sent as given
    async fn update(&self, wire: &WriteWire, asynchronous: bool) -> Result<Json, CallError> {
        let provider = ensure_provider(wire.custom_llm_provider.as_deref()).to_owned();
        let logging = Mutex::new(self.logging_state("update").map_err(|source| CallError {
            provider: provider.clone(),
            source: CallFailure::Write(Box::new(write_failure(
                VectorFileWriteError::Logging(source),
                Some(&provider),
                &self.redactor,
            ))),
        })?);
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
        let request = update_request(wire);
        let answer = if asynchronous {
            writes.aupdate(&request).await
        } else {
            writes.update(&request).await
        };
        answer.map_err(|source| CallError {
            provider,
            source: CallFailure::Write(Box::new(source)),
        })
    }

    // B1074's delete: the object's id and the deletion answer, with no body or query of its own
    async fn delete(&self, wire: &WriteWire, asynchronous: bool) -> Result<Json, CallError> {
        let provider = ensure_provider(wire.custom_llm_provider.as_deref()).to_owned();
        let logging = Mutex::new(self.logging_state("delete").map_err(|source| CallError {
            provider: provider.clone(),
            source: CallFailure::Write(Box::new(write_failure(
                VectorFileWriteError::Logging(source),
                Some(&provider),
                &self.redactor,
            ))),
        })?);
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
        let request = write_request(wire);
        let answer = if asynchronous {
            writes.adelete(&request).await
        } else {
            writes.delete(&request).await
        };
        answer.map_err(|source| CallError {
            provider,
            source: CallFailure::Write(Box::new(source)),
        })
    }
}
