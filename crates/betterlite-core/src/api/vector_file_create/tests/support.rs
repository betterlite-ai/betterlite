use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::vector_file_contracts::records::{Attributes, FileParams};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::super::service::{FileCall, FileService, Operation};
use super::super::{
    CreateFileRequest, FileResult, ListFileRequest, TransportRequest, VectorFileCreateDeps,
    VectorFileCreates,
};
use crate::api::vector_file_http_create_list::config::FilesParams;
use crate::api::vector_file_http_create_list::error::FileTransportError;
use crate::api::vector_file_provider::{CredentialLookup, CredentialLookupError, OperationKwargs};
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};
use crate::api::vector_store_http_write::service::ClientKind;
use crate::api::vector_store_search::error::SdkFailure;

pub(crate) const CALL_ID: &str = "call-a";
pub(crate) const VECTOR_STORE_ID: &str = "vs_a";
pub(crate) const API_BASE: &str = "https://api.openai.com/v1";
pub(crate) const REQUEST_TIMEOUT: f64 = 6000.0;
pub(crate) const FILE_BODY: &str =
    r#"{"id":"file_b","object":"vector_store.file","status":"in_progress"}"#;
pub(crate) const LIST_BODY: &str =
    r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;

pub(crate) fn file() -> Json {
    serde_json::from_str(FILE_BODY).expect("the file reply is JSON")
}

pub(crate) fn listing() -> Json {
    serde_json::from_str(LIST_BODY).expect("the list reply is JSON")
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(crate) fn attributes(text: &str) -> Attributes {
    serde_json::from_str(text).expect("the attributes fixture is JSON")
}

pub(crate) fn chunking() -> IndexMap<String, Json> {
    serde_json::from_str(
        r#"{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}}"#,
    )
    .expect("the chunking fixture is JSON")
}

pub(crate) fn params(text: &str) -> FileParams {
    serde_json::from_str(text).expect("the parameter fixture is JSON")
}

pub(crate) fn object(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the overlay fixture is JSON")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the value fixture is JSON")
}

// a logger function_setup built for a call with no model: model stays empty, betterlite's null
pub(crate) fn logging() -> LoggingState {
    let text = format!(
        r#"{{"model":"","call_type":"endpoint_func","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"{CALL_ID}","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
    );
    let init: LoggingInit = serde_json::from_str(&text).expect("the logging fixture is JSON");
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || String::from("generated-trace"),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

// the registry B1111 overlays: one store's credential values, or a lookup that raises
pub(crate) struct Registry {
    pub(crate) values: IndexMap<String, Json>,
    pub(crate) failing: bool,
}

impl Registry {
    pub(crate) fn new(values: IndexMap<String, Json>) -> Self {
        Self {
            values,
            failing: false,
        }
    }
}

impl CredentialLookup for Registry {
    fn lookup_credentials(
        &self,
        vector_store_id: &str,
    ) -> Result<OperationKwargs, CredentialLookupError> {
        if self.failing {
            return Err(CredentialLookupError::Unavailable {
                reason: String::from("the registry is down"),
            });
        }
        if vector_store_id != VECTOR_STORE_ID {
            return Ok(OperationKwargs(IndexMap::new()));
        }
        Ok(OperationKwargs(self.values.clone()))
    }
}

// the SDK's named arguments and kwargs, with every optional one at its declared default
pub(crate) struct Fixture {
    pub(crate) vector_store_id: String,
    pub(crate) file_id: String,
    pub(crate) attributes: Option<Attributes>,
    pub(crate) chunking_strategy: Option<IndexMap<String, Json>>,
    pub(crate) after: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) filter: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    pub(crate) timeout: Option<f64>,
    pub(crate) provider: Option<String>,
    pub(crate) kwargs: IndexMap<String, Json>,
    pub(crate) logging: Mutex<LoggingState>,
    pub(crate) redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self {
            vector_store_id: String::from(VECTOR_STORE_ID),
            file_id: String::from("file_a"),
            attributes: None,
            chunking_strategy: None,
            after: None,
            before: None,
            filter: None,
            limit: None,
            order: None,
            extra_headers: None,
            extra_query: None,
            extra_body: None,
            timeout: None,
            provider: None,
            kwargs: IndexMap::new(),
            logging: Mutex::new(logging()),
            redactor: redactor(),
        }
    }

    pub(crate) fn transport(&self) -> TransportRequest<'_> {
        TransportRequest {
            vector_store_id: &self.vector_store_id,
            extra_headers: self.extra_headers.as_ref(),
            extra_query: self.extra_query.as_ref(),
            extra_body: self.extra_body.as_ref(),
            timeout: self.timeout,
            custom_llm_provider: self.provider.as_deref(),
            kwargs: &self.kwargs,
        }
    }

    fn create_request<'fixture>(
        &'fixture self,
        transport: TransportRequest<'fixture>,
    ) -> CreateFileRequest<'fixture> {
        CreateFileRequest {
            transport,
            file_id: &self.file_id,
            attributes: self.attributes.as_ref(),
            chunking_strategy: self.chunking_strategy.as_ref(),
        }
    }

    fn list_request<'fixture>(
        &'fixture self,
        transport: TransportRequest<'fixture>,
    ) -> ListFileRequest<'fixture> {
        ListFileRequest {
            transport,
            after: self.after.as_deref(),
            before: self.before.as_deref(),
            filter: self.filter.as_deref(),
            limit: self.limit,
            order: self.order.as_deref(),
        }
    }
}

// which public entry point runs; the sync forms still let an acreate/alist flag in kwargs select async
#[derive(Clone, Copy)]
pub(crate) enum Which {
    Create,
    Acreate,
    List,
    Alist,
}

pub(crate) async fn call(
    which: Which,
    spy: &Spy,
    fixture: &Fixture,
    registry: Option<&dyn CredentialLookup>,
    request_timeout_seconds: f64,
) -> FileResult {
    let deps = VectorFileCreateDeps {
        logging: &fixture.logging,
        redactor: &fixture.redactor,
        registry,
        request_timeout_seconds,
    };
    let files = VectorFileCreates { service: spy, deps };
    let transport = fixture.transport();
    match which {
        Which::Create => files.create(&fixture.create_request(transport)).await,
        Which::Acreate => files.acreate(&fixture.create_request(transport)).await,
        Which::List => files.list(&fixture.list_request(transport)).await,
        Which::Alist => files.alist(&fixture.list_request(transport)).await,
    }
}

pub(crate) async fn recorded(
    which: Which,
    spy: &Spy,
    fixture: &Fixture,
    registry: Option<&dyn CredentialLookup>,
    request_timeout_seconds: f64,
) -> Recorded {
    call(which, spy, fixture, registry, request_timeout_seconds)
        .await
        .expect("the attachment call succeeds");
    spy.seen().expect("the service saw the call")
}

pub(crate) async fn refused(
    which: Which,
    spy: &Spy,
    fixture: &Fixture,
    registry: Option<&dyn CredentialLookup>,
) -> SdkFailure {
    call(which, spy, fixture, registry, REQUEST_TIMEOUT)
        .await
        .expect_err("the attachment call fails")
}

// what the service saw: the call's fields and the logging state the call left behind
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) operation: Operation,
    pub(crate) mode: ClientKind,
    pub(crate) provider: ProviderId,
    pub(crate) config: VectorStoreFilesConfigTag,
    pub(crate) vector_store_id: String,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) ssl_verify: Option<bool>,
    pub(crate) fields: FileParams,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    pub(crate) timeout: Duration,
    pub(crate) model: String,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) custom_llm_provider: Option<String>,
}

fn observe(file_call: &FileCall<'_>) -> Recorded {
    let state = file_call
        .logging
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    Recorded {
        operation: file_call.operation,
        mode: file_call.mode,
        provider: file_call.provider,
        config: file_call.config,
        vector_store_id: file_call.vector_store_id.to_owned(),
        api_base: file_call.params.api_base.clone(),
        api_key: file_call
            .params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        ssl_verify: file_call.params.ssl_verify,
        fields: file_call.fields.clone(),
        extra_headers: file_call.extra_headers.cloned(),
        extra_body: file_call.extra_body.cloned(),
        extra_query: file_call.extra_query.cloned(),
        timeout: file_call.timeout,
        model: state.model.clone(),
        optional_params: state.optional_params.clone(),
        litellm_params: state.litellm_params.clone(),
        custom_llm_provider: state.custom_llm_provider.clone(),
    }
}

enum Reply {
    Value(Json),
    Provider { status: u16, message: String },
}

pub(crate) struct Spy {
    reply: Mutex<Reply>,
    seen: Mutex<Option<Recorded>>,
    calls: AtomicUsize,
}

impl Spy {
    pub(crate) fn replying(value: Json) -> Self {
        Self {
            reply: Mutex::new(Reply::Value(value)),
            seen: Mutex::new(None),
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn failing(status: u16, message: &str) -> Self {
        Self {
            reply: Mutex::new(Reply::Provider {
                status,
                message: message.to_owned(),
            }),
            seen: Mutex::new(None),
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn seen(&self) -> Option<Recorded> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // how many calls reached the service: a stray second delegation shows up here, not only in the last Recorded
    pub(crate) fn call_count(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    fn answer(&self) -> Result<Json, FileTransportError> {
        let reply = self.reply.lock().unwrap_or_else(PoisonError::into_inner);
        match &*reply {
            Reply::Value(value) => Ok(value.clone()),
            Reply::Provider { status, message } => {
                Err(FileTransportError::Provider(BaseLlmException {
                    status_code: *status,
                    message: message.clone(),
                    headers: HeaderMap::new(),
                    status_code_is_synthesized: false,
                }))
            }
        }
    }
}

impl FileService for Spy {
    fn file(
        &self,
        file_call: &FileCall<'_>,
    ) -> impl Future<Output = Result<Json, FileTransportError>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(observe(file_call));
        ready(self.answer())
    }
}
