use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_file_contracts::records::Attributes;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::super::error::VectorFileWriteFailure;
use super::super::service::{WriteCall, WriteService};
use super::super::{FileUpdateRequest, FileWriteRequest, VectorFileWriteDeps, VectorFileWrites};
use crate::api::vector_file_http_write::error::FileWriteError;
use crate::api::vector_file_provider::{CredentialLookup, CredentialLookupError, OperationKwargs};
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};
use crate::api::vector_store_http_write::service::{ClientKind, Operation};

pub(crate) const VECTOR_STORE_ID: &str = "vs_a";
pub(crate) const FILE_ID: &str = "file_a";
pub(crate) const REQUEST_TIMEOUT: f64 = 6000.0;
pub(crate) const SUCCESS_BODY: &str =
    r#"{"id": "file_a", "object": "vector_store.file", "attributes": {"replacement": "y"}}"#;
pub(crate) const DELETED_BODY: &str =
    r#"{"id": "file_a", "object": "vector_store.file.deleted", "deleted": true}"#;
pub(crate) const PROVIDER_ERROR: &str = r#"{"error": {"message": "missing", "type": "invalid_request_error", "param": null, "code": "missing"}}"#;

pub(crate) fn success() -> Json {
    serde_json::from_str(SUCCESS_BODY).expect("the success reply is JSON")
}

pub(crate) fn deleted() -> Json {
    serde_json::from_str(DELETED_BODY).expect("the delete reply is JSON")
}

// the argument before B527's cleaning, as the logging state keeps it
pub(crate) const ATTRIBUTES_BODY: &str = r#"{"tag": "x", "count": 2, "enabled": false}"#;

pub(crate) fn attributes() -> Attributes {
    serde_json::from_str(ATTRIBUTES_BODY).expect("the attributes are typed")
}

pub(crate) fn attributes_json() -> Json {
    serde_json::from_str(ATTRIBUTES_BODY).expect("the attributes are JSON")
}

pub(crate) fn replacement_body() -> IndexMap<String, Json> {
    IndexMap::from([(
        String::from("attributes"),
        Json::Object(IndexMap::from([(
            String::from("replacement"),
            Json::Text(String::from("y")),
        )])),
    )])
}

pub(crate) fn kept_headers() -> BTreeMap<String, String> {
    BTreeMap::from([(String::from("X-Trace"), String::from("t"))])
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

// a logger function_setup built for a call with no model: model stays empty, betterlite's null
pub(crate) fn logging() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: String::new(),
            call_type: String::from("vector_store_file_update"),
            start_time: moment(),
            litellm_call_id: String::from("call-1"),
            function_id: String::from("fn-1"),
            messages: None,
            stream: None,
            litellm_trace_id: Some(String::from("trace-1")),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
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

// The injectable credential store: values replace the caller's, and a raising lookup is the one a caller survives
pub(crate) struct Registry {
    credentials: Option<JsonObject>,
    lookups: AtomicUsize,
}

impl Registry {
    pub(crate) fn replying(values: JsonObject) -> Self {
        Self {
            credentials: Some(values),
            lookups: AtomicUsize::new(0),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            credentials: None,
            lookups: AtomicUsize::new(0),
        }
    }

    pub(crate) fn lookup_count(&self) -> usize {
        self.lookups.load(Ordering::Relaxed)
    }
}

impl CredentialLookup for Registry {
    fn lookup_credentials(
        &self,
        _vector_store_id: &str,
    ) -> Result<OperationKwargs, CredentialLookupError> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        match &self.credentials {
            Some(values) => Ok(OperationKwargs(values.clone())),
            None => Err(CredentialLookupError::Unavailable {
                reason: String::from("the registry is unreachable"),
            }),
        }
    }
}

// the SDK's named arguments and kwargs, with every optional one at its declared default
#[derive(Default)]
pub(crate) struct Arguments {
    pub(crate) attributes: Option<Attributes>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    pub(crate) timeout: Option<f64>,
    pub(crate) provider: Option<String>,
    pub(crate) kwargs: IndexMap<String, Json>,
}

pub(crate) struct Fixture {
    pub(crate) arguments: Arguments,
    pub(crate) logging: Mutex<LoggingState>,
    pub(crate) registry: Option<Registry>,
    pub(crate) redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self {
            arguments: Arguments::default(),
            logging: Mutex::new(logging()),
            registry: None,
            redactor: redactor(),
        }
    }

    fn write_request(&self) -> FileWriteRequest<'_> {
        FileWriteRequest {
            vector_store_id: VECTOR_STORE_ID,
            file_id: FILE_ID,
            extra_headers: self.arguments.extra_headers.as_ref(),
            timeout: self.arguments.timeout,
            custom_llm_provider: self.arguments.provider.as_deref(),
            kwargs: &self.arguments.kwargs,
        }
    }

    fn update_request(&self) -> FileUpdateRequest<'_> {
        FileUpdateRequest {
            write: self.write_request(),
            attributes: self.arguments.attributes.as_ref(),
            extra_body: self.arguments.extra_body.as_ref(),
        }
    }

    fn deps(&self, request_timeout_seconds: f64) -> VectorFileWriteDeps<'_> {
        let mut deps = VectorFileWriteDeps {
            logging: &self.logging,
            registry: None,
            redactor: &self.redactor,
            request_timeout_seconds,
        };
        if let Some(registry) = &self.registry {
            deps.registry = Some(registry);
        }
        deps
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Which {
    Update,
    Aupdate,
    Delete,
    Adelete,
}

pub(crate) async fn call(
    which: Which,
    spy: &Spy,
    fixture: &Fixture,
    request_timeout_seconds: f64,
) -> Result<Json, VectorFileWriteFailure> {
    let deps = fixture.deps(request_timeout_seconds);
    let writes = VectorFileWrites { service: spy, deps };
    match which {
        Which::Update => writes.update(&fixture.update_request()).await,
        Which::Aupdate => writes.aupdate(&fixture.update_request()).await,
        Which::Delete => writes.delete(&fixture.write_request()).await,
        Which::Adelete => writes.adelete(&fixture.write_request()).await,
    }
}

pub(crate) async fn recorded(
    which: Which,
    spy: &Spy,
    fixture: &Fixture,
    request_timeout_seconds: f64,
) -> Recorded {
    call(which, spy, fixture, request_timeout_seconds)
        .await
        .expect("the write succeeds");
    spy.seen().expect("the service saw the write")
}

// what the service saw: the call's fields and the logging state the write left behind
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub(crate) operation: Operation,
    pub(crate) mode: ClientKind,
    pub(crate) provider: ProviderId,
    pub(crate) config: VectorStoreFilesConfigTag,
    pub(crate) vector_store_id: String,
    pub(crate) file_id: String,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) ssl_verify: Option<bool>,
    pub(crate) update: String,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    pub(crate) timeout: Duration,
    pub(crate) model: String,
    pub(crate) optional: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) details: IndexMap<String, Json>,
    pub(crate) provider_name: Option<String>,
}

fn observe(call: &WriteCall<'_>) -> Recorded {
    let state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
    Recorded {
        operation: call.operation,
        mode: call.mode,
        provider: call.provider,
        config: call.config,
        vector_store_id: call.vector_store_id.to_owned(),
        file_id: call.file_id.to_owned(),
        api_base: call.params.api_base.clone(),
        api_key: call
            .params
            .api_key
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        ssl_verify: call.params.ssl_verify,
        update: serde_json::to_string(call.update_request).expect("the update request encodes"),
        extra_headers: call.extra_headers.cloned(),
        extra_body: call.extra_body.cloned(),
        timeout: call.timeout,
        model: state.model.clone(),
        optional: state.optional_params.clone(),
        litellm_params: state.litellm_params.clone(),
        details: state.model_call_details.clone(),
        provider_name: state.custom_llm_provider.clone(),
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

    // how many writes reached the service: a stray second delegation shows up here, not only in the last Recorded
    pub(crate) fn call_count(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    fn answer(&self) -> Result<Json, FileWriteError> {
        let reply = self.reply.lock().unwrap_or_else(PoisonError::into_inner);
        match &*reply {
            Reply::Value(value) => Ok(value.clone()),
            Reply::Provider { status, message } => {
                Err(FileWriteError::Provider(BaseLlmException {
                    status_code: *status,
                    message: message.clone(),
                    headers: HeaderMap::new(),
                    status_code_is_synthesized: false,
                }))
            }
        }
    }
}

impl WriteService for Spy {
    fn write(
        &self,
        call: &WriteCall<'_>,
    ) -> impl Future<Output = Result<Json, FileWriteError>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(observe(call));
        ready(self.answer())
    }
}
