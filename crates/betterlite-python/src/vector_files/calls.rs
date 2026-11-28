use std::sync::Mutex;

use betterlite_core::api::vector_file_create::error::FileCallError;
use betterlite_core::api::vector_file_create::error::failure as file_failure;
use betterlite_core::api::vector_file_create::{VectorFileCreateDeps, VectorFileCreates};
use betterlite_core::api::vector_file_http_read::service::LogContext;
use betterlite_core::api::vector_file_provider::ensure_provider;
use betterlite_core::api::vector_file_read::error::VectorFileReadError;
use betterlite_core::api::vector_file_read::service::{ReadSession, SdkDeps};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;

use super::convert::list::list_request;
use super::convert::read::read_call;
use super::convert::{CallError, CallFailure, CreateWire, ListWire, ReadWire, create_request};
use super::session::{Pending, PendingOperation, SessionCore};
use super::source::SilentSink;
use super::write_calls::{WriteSide, write_call};

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
    const fn call_type(&self) -> &'static str {
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

// Every native call's operation, with the provider its failure names and the family error the service gave
pub(crate) async fn run(core: &SessionCore, pending: Pending) -> Result<Json, CallError> {
    let asynchronous = pending.asynchronous;
    match pending.operation {
        PendingOperation::Create(wire) => {
            file_call(core, FileSide::Create(&wire), asynchronous).await
        }
        PendingOperation::List(wire) => file_call(core, FileSide::List(&wire), asynchronous).await,
        PendingOperation::Retrieve(wire) => {
            read(core, &wire, ReadSide::Retrieve, asynchronous).await
        }
        PendingOperation::RetrieveContent(wire) => {
            read(core, &wire, ReadSide::Content, asynchronous).await
        }
        PendingOperation::Update(wire) => {
            write_call(core, &wire, WriteSide::Update, asynchronous).await
        }
        PendingOperation::Delete(wire) => {
            write_call(core, &wire, WriteSide::Delete, asynchronous).await
        }
    }
}

// B1072's create and list: one prepared session each, and the flag picks the async handler
async fn file_call(
    core: &SessionCore,
    side: FileSide<'_>,
    asynchronous: bool,
) -> Result<Json, CallError> {
    let provider = ensure_provider(side.provider()).to_owned();
    let logging = file_logging(core, &side, &provider)?;
    let deps = VectorFileCreateDeps {
        logging: &logging,
        redactor: &core.redactor,
        registry: None,
        request_timeout_seconds: core.request_timeout_seconds,
    };
    let creates = VectorFileCreates {
        service: core,
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

// The create or list pre-call state, with the family error a logging failure carries
fn file_logging(
    core: &SessionCore,
    side: &FileSide<'_>,
    provider: &str,
) -> Result<Mutex<LoggingState>, CallError> {
    let state = core
        .logging_state(side.call_type())
        .map_err(|source| CallError {
            provider: provider.to_owned(),
            source: CallFailure::File(file_failure(
                &FileCallError::Logging(source),
                provider,
                &core.redactor,
            )),
        })?;
    Ok(Mutex::new(state))
}

// B1073's retrieve or content retrieve, with the read session holding the configs and the client
async fn read(
    core: &SessionCore,
    wire: &ReadWire,
    side: ReadSide,
    asynchronous: bool,
) -> Result<Json, CallError> {
    let provider = ensure_provider(wire.custom_llm_provider.as_deref()).to_owned();
    let mut state = core
        .logging_state(side.call_type())
        .map_err(|source| CallError {
            provider: provider.clone(),
            source: CallFailure::Read(VectorFileReadError::Logging(source)),
        })?;
    let call = read_call(wire);
    let deps = SdkDeps {
        configs: &core.configs,
        registry: None,
        source: &core.source,
        supplied: None,
        request_timeout_seconds: core.request_timeout_seconds,
    };
    let sink = SilentSink;
    let env = core.pre_post(&sink);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let mut session = ReadSession {
        deps: &deps,
        call: &call,
        log: &mut log,
    };
    let answer = match (side, asynchronous) {
        (ReadSide::Retrieve, true) => session.aretrieve().await,
        (ReadSide::Retrieve, false) => session.retrieve().await,
        (ReadSide::Content, true) => session.aretrieve_content().await,
        (ReadSide::Content, false) => session.retrieve_content().await,
    };
    answer.map_err(|source| CallError {
        provider,
        source: CallFailure::Read(source),
    })
}

mod tests;
