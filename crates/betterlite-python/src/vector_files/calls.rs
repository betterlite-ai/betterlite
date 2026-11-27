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

// Which create-family call a wire stands for, so create and list share one prepared session
enum FileSide<'wire> {
    Create(&'wire CreateWire),
    List(&'wire ListWire),
}

impl FileSide<'_> {
    fn call_type(&self) -> &'static str {
        match self {
            Self::Create(_wire) => "create",
            Self::List(_wire) => "list",
        }
    }

    fn provider(&self) -> Option<&str> {
        match self {
            Self::Create(wire) => wire.custom_llm_provider.as_deref(),
            Self::List(wire) => wire.custom_llm_provider.as_deref(),
        }
    }
}

// Which write a wire stands for: update carries a body, delete does not
#[derive(Clone, Copy)]
enum WriteSide {
    Update,
    Delete,
}

impl WriteSide {
    const fn call_type(self) -> &'static str {
        match self {
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

// Everything one native call runs, with the provider its failure names and the family error the service gave
impl SessionCore {
    pub(crate) async fn run(&self, pending: Pending) -> Result<Json, CallError> {
        let asynchronous = pending.asynchronous;
        match pending.operation {
            PendingOperation::Create(wire) => {
                self.file_call(FileSide::Create(&wire), asynchronous).await
            }
            PendingOperation::List(wire) => {
                self.file_call(FileSide::List(&wire), asynchronous).await
            }
            PendingOperation::Retrieve(wire) => {
                self.read(&wire, ReadSide::Retrieve, asynchronous).await
            }
            PendingOperation::RetrieveContent(wire) => {
                self.read(&wire, ReadSide::Content, asynchronous).await
            }
            PendingOperation::Update(wire) => {
                self.write_call(&wire, WriteSide::Update, asynchronous)
                    .await
            }
            PendingOperation::Delete(wire) => {
                self.write_call(&wire, WriteSide::Delete, asynchronous)
                    .await
            }
        }
    }

    // B1072's create and list: one prepared session each, and the flag picks the async handler
    async fn file_call(&self, side: FileSide<'_>, asynchronous: bool) -> Result<Json, CallError> {
        let provider = ensure_provider(side.provider()).to_owned();
        let logging =
            Mutex::new(
                self.logging_state(side.call_type())
                    .map_err(|source| CallError {
                        provider: provider.clone(),
                        source: CallFailure::File(file_failure(
                            &FileCallError::Logging(source),
                            &provider,
                            &self.redactor,
                        )),
                    })?,
            );
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
        let answer = match side {
            FileSide::Create(wire) => {
                let request = create_request(wire);
                if asynchronous {
                    creates.acreate(&request).await
                } else {
                    creates.create(&request).await
                }
            }
            FileSide::List(wire) => {
                let request = list_request(wire);
                if asynchronous {
                    creates.alist(&request).await
                } else {
                    creates.list(&request).await
                }
            }
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

    // B1074's update and delete: one prepared write session, and the flag picks the async handler
    async fn write_call(
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
