#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::vector_store_records::records::vector::{
    AutoChunkingStrategy, VectorStoreChunkingStrategy, VectorStoreExpirationPolicy,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde_json::Number;

use super::super::error::VectorStoreWriteFailure;
use super::super::service::{Operation, WriteCall, WriteService};
use super::super::{
    TransportRequest, VectorStoreWriteDeps, VectorStoreWriteRequest, VectorStoreWrites,
};
use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap, WriteError};
use crate::api::vector_store_http_write::params::UpdateParams;
use crate::api::vector_store_http_write::service::ClientKind;

pub(crate) const CALL_ID: &str = "call-1";
pub(crate) const VECTOR_STORE_ID: &str = "vs_a";
pub(crate) const API_BASE: &str = "https://api.openai.com/v1";
pub(crate) const REQUEST_TIMEOUT: f64 = 6000.0;
pub(crate) const SUCCESS_BODY: &str =
    r#"{"id": "vs_a", "object": "vector_store", "name": "notes"}"#;
pub(crate) const DELETED_BODY: &str =
    r#"{"id": "vs_a", "object": "vector_store.deleted", "deleted": true}"#;
pub(crate) const PROVIDER_ERROR: &str = r#"{"error": {"message": "vector store not found", "type": "invalid_request_error", "param": null, "code": "missing"}}"#;

pub(crate) fn success() -> Json {
    serde_json::from_str(SUCCESS_BODY).expect("the success reply is JSON")
}

pub(crate) fn deleted() -> Json {
    serde_json::from_str(DELETED_BODY).expect("the delete reply is JSON")
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
            call_type: String::from("endpoint_func"),
            start_time: moment(),
            litellm_call_id: String::from(CALL_ID),
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

pub(crate) struct Resolution {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Resolution {
    pub(crate) fn new() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }
}

pub(crate) fn expires_after() -> VectorStoreExpirationPolicy {
    VectorStoreExpirationPolicy {
        anchor: Some(String::from("last_active_at")),
        days: Some(3),
        extra: BTreeMap::new(),
    }
}

pub(crate) fn auto_chunking() -> VectorStoreChunkingStrategy {
    VectorStoreChunkingStrategy::Auto(AutoChunkingStrategy {
        type_name: Some(String::from("auto")),
        extra: BTreeMap::new(),
    })
}

pub(crate) fn metadata() -> IndexMap<String, Json> {
    let mut map = IndexMap::new();
    map.insert(String::from("owner"), Json::Text(String::from("alice")));
    map.insert(String::from("count"), Json::Number(Number::from(2_u32)));
    map
}

pub(crate) fn kept_headers() -> BTreeMap<String, String> {
    BTreeMap::from([(String::from("X-Trace"), String::from("kept"))])
}

pub(crate) fn lost_query() -> IndexMap<String, Json> {
    IndexMap::from([(String::from("trace"), Json::Text(String::from("lost")))])
}

pub(crate) fn replacement_body() -> IndexMap<String, Json> {
    IndexMap::from([(
        String::from("name"),
        Json::Text(String::from("replacement")),
    )])
}

// the SDK's named arguments and kwargs, with every optional one at its declared default of null
#[derive(Default)]
pub(crate) struct Arguments {
    pub(crate) name: Option<String>,
    pub(crate) expires_after: Option<VectorStoreExpirationPolicy>,
    pub(crate) metadata: Option<IndexMap<String, Json>>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    pub(crate) timeout: Option<f64>,
    pub(crate) provider: Option<String>,
    pub(crate) kwargs: IndexMap<String, Json>,
}

// one public write call's arguments and the collaborators the production service would receive
pub(crate) struct Fixture {
    pub(crate) arguments: Arguments,
    pub(crate) params: WriteParams,
    pub(crate) logging: Mutex<LoggingState>,
    pub(crate) redactor: Redactor,
}

fn write_params() -> WriteParams {
    WriteParams {
        api_base: Some(String::from(API_BASE)),
        api_key: None,
        ssl_verify: None,
    }
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self {
            arguments: Arguments::default(),
            params: write_params(),
            logging: Mutex::new(logging()),
            redactor: redactor(),
        }
    }

    pub(crate) fn request(&self) -> VectorStoreWriteRequest<'_> {
        VectorStoreWriteRequest {
            transport: TransportRequest {
                vector_store_id: VECTOR_STORE_ID,
                extra_headers: self.arguments.extra_headers.as_ref(),
                extra_query: self.arguments.extra_query.as_ref(),
                extra_body: self.arguments.extra_body.as_ref(),
                timeout: self.arguments.timeout,
                custom_llm_provider: self.arguments.provider.as_deref(),
                params: &self.params,
                kwargs: &self.arguments.kwargs,
            },
            name: self.arguments.name.as_deref(),
            expires_after: self.arguments.expires_after.as_ref(),
            metadata: self.arguments.metadata.as_ref(),
        }
    }
}

pub(crate) fn write_deps<'deps>(
    fixture: &'deps Fixture,
    resolution: &'deps ResolutionContext<'deps>,
    request_timeout_seconds: f64,
) -> VectorStoreWriteDeps<'deps> {
    VectorStoreWriteDeps {
        logging: &fixture.logging,
        redactor: &fixture.redactor,
        resolution,
        request_timeout_seconds,
    }
}

// which public entry point runs; the sync forms still let an aupdate/adelete flag in kwargs select async
#[derive(Clone, Copy)]
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
    resolution: &Resolution,
    request_timeout_seconds: f64,
) -> Result<Json, VectorStoreWriteFailure> {
    let context = ResolutionContext {
        catalog: &resolution.catalog,
        known_model_names: &resolution.names,
        custom_providers: &resolution.names,
        environment: &resolution.environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let deps = write_deps(fixture, &context, request_timeout_seconds);
    let writes = VectorStoreWrites { service: spy, deps };
    let request = fixture.request();
    match which {
        Which::Update => writes.update(&request).await,
        Which::Aupdate => writes.aupdate(&request).await,
        Which::Delete => writes.delete(&request).await,
        Which::Adelete => writes.adelete(&request).await,
    }
}

pub(crate) async fn recorded(
    which: Which,
    spy: &Spy,
    fixture: &Fixture,
    resolution: &Resolution,
    request_timeout_seconds: f64,
) -> Recorded {
    call(which, spy, fixture, resolution, request_timeout_seconds)
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
    pub(crate) config: VectorStoresConfigTag,
    pub(crate) vector_store_id: String,
    pub(crate) api_base: Option<String>,
    pub(crate) optional_params: UpdateParams,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    pub(crate) timeout: Duration,
    pub(crate) model: String,
    pub(crate) call_type: String,
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
        api_base: call.params.api_base.clone(),
        optional_params: call.optional_params.clone(),
        extra_headers: call.extra_headers.cloned(),
        extra_body: call.extra_body.cloned(),
        timeout: call.timeout,
        model: state.model.clone(),
        call_type: state.call_type.clone(),
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

    fn answer(&self) -> Result<Json, WriteError> {
        let reply = self.reply.lock().unwrap_or_else(PoisonError::into_inner);
        match &*reply {
            Reply::Value(value) => Ok(value.clone()),
            Reply::Provider { status, message } => Err(WriteError::Provider(BaseLlmException {
                status_code: *status,
                message: message.clone(),
                headers: HeaderMap::new(),
                status_code_is_synthesized: false,
            })),
        }
    }
}

impl WriteService for Spy {
    fn write(&self, call: &WriteCall<'_>) -> impl Future<Output = Result<Json, WriteError>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(observe(call));
        ready(self.answer())
    }
}
