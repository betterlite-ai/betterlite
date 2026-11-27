#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::error::VectorStoreReadFailure;
use super::super::service::{
    ListQuery, Operation, ReadCall, ReadConfig, ReadProvider, VectorStoreReadConfigs,
};
use super::super::{
    TransportRequest, VectorStoreListRequest, VectorStoreReadDeps, VectorStoreReadRequest,
    VectorStoreReads,
};
use crate::api::vector_store_http_create_read::config::OpenAiCreateReadConfig;
use crate::api::vector_store_http_create_read::error::CreateReadError;
use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap, WriteError};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use crate::api::vector_store_search::provider::ResolvedProvider;

pub(crate) const VECTOR_STORE_ID: &str = "vs_a";
pub(crate) const API_BASE: &str = "https://api.openai.com/v1";
pub(crate) const REQUEST_TIMEOUT: f64 = 6000.0_f64;
const RECORD_BODY: &str = r#"{"id":"vs_a","object":"vector_store","name":"notes","status":"completed","file_counts":{"total":2,"completed":2}}"#;
const LIST_BODY: &str =
    r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;

pub(crate) fn record() -> Json {
    serde_json::from_str(RECORD_BODY).expect("the record reply is JSON")
}

pub(crate) fn listing() -> Json {
    serde_json::from_str(LIST_BODY).expect("the list reply is JSON")
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(crate) fn openai_settings() -> OpenAiCredentialsSettings {
    OpenAiCredentialsSettings::default()
}

pub(crate) fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

// LiteLLM starts an SDK read from model "vector-store" and the operation's call_type
pub(crate) fn logging(call_type: &str) -> LoggingState {
    let text = format!(
        r#"{{"model":"vector-store","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
    );
    let init: LoggingInit = serde_json::from_str(&text).expect("the logging fixture is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SeenProvider {
    pub(crate) provider: ProviderId,
    pub(crate) api_type: Option<String>,
    pub(crate) tag: VectorStoresConfigTag,
}

// B010's source: what the resolved provider and api_type select, and what it was asked for
pub(crate) struct Configs {
    settings: OpenAiCredentialsSettings,
    served: BTreeSet<ProviderId>,
    seen: Mutex<Vec<SeenProvider>>,
}

impl Configs {
    pub(crate) fn openai() -> Self {
        Self::serving(BTreeSet::from([ProviderId::Openai]))
    }

    pub(crate) fn serving(served: BTreeSet<ProviderId>) -> Self {
        Self {
            settings: openai_settings(),
            served,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn seen(&self) -> Vec<SeenProvider> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl VectorStoreReadConfigs for Configs {
    fn read_config<'source>(
        &'source self,
        resolved: &ResolvedProvider,
    ) -> Option<ReadConfig<'source>> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SeenProvider {
                provider: resolved.provider,
                api_type: resolved.api_type.clone(),
                tag: resolved.tag,
            });
        if !self.served.contains(&resolved.provider) {
            return None;
        }
        Some(ReadConfig(Box::new(OpenAiCreateReadConfig::new(
            &self.settings,
            LiteLlmGlobals::default(),
            Some(SecretString::from("sk-env")),
            None,
        ))))
    }
}

// the SDK's named arguments and kwargs, with every optional one at its declared default
#[derive(Default)]
pub(crate) struct Arguments {
    pub(crate) provider: Option<String>,
    pub(crate) vector_store_id: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    pub(crate) timeout: Option<f64>,
    pub(crate) after: Option<String>,
    pub(crate) before: Option<String>,
    // absent, present and null, or a value
    pub(crate) limit: Option<Option<i64>>,
    pub(crate) order: Option<Option<String>>,
    pub(crate) kwargs: IndexMap<String, Json>,
}

// one public read's arguments and the collaborators the production service would receive
pub(crate) struct Fixture {
    pub(crate) arguments: Arguments,
    pub(crate) params: WriteParams,
    pub(crate) logging: Mutex<LoggingState>,
    pub(crate) redactor: Redactor,
    pub(crate) configs: Configs,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self::with_configs(Configs::openai())
    }

    pub(crate) fn with_configs(configs: Configs) -> Self {
        Self {
            arguments: Arguments::default(),
            params: WriteParams {
                api_base: Some(String::from(API_BASE)),
                api_key: None,
                ssl_verify: None,
            },
            logging: Mutex::new(logging("endpoint_func")),
            redactor: redactor(),
            configs,
        }
    }

    fn transport(&self) -> TransportRequest<'_> {
        TransportRequest {
            extra_headers: self.arguments.extra_headers.as_ref(),
            extra_query: self.arguments.extra_query.as_ref(),
            extra_body: self.arguments.extra_body.as_ref(),
            timeout: self.arguments.timeout,
            custom_llm_provider: self.arguments.provider.as_deref(),
            params: &self.params,
            kwargs: &self.arguments.kwargs,
        }
    }

    pub(crate) fn request(&self) -> VectorStoreReadRequest<'_> {
        VectorStoreReadRequest {
            transport: self.transport(),
            vector_store_id: self
                .arguments
                .vector_store_id
                .as_deref()
                .unwrap_or(VECTOR_STORE_ID),
        }
    }

    pub(crate) fn list_request(&self) -> VectorStoreListRequest<'_> {
        VectorStoreListRequest {
            transport: self.transport(),
            after: self.arguments.after.as_deref(),
            before: self.arguments.before.as_deref(),
            limit: self.arguments.limit,
            order: self.arguments.order.as_ref().map(|order| order.as_deref()),
        }
    }
}

// which public entry point runs; the sync forms still let an aretrieve/alist flag in kwargs select async
#[derive(Clone, Copy)]
pub(crate) enum Which {
    Retrieve,
    Aretrieve,
    List,
    Alist,
}

impl Which {
    pub(crate) const fn mode(self) -> ClientKind {
        match self {
            Self::Retrieve | Self::List => ClientKind::Sync,
            Self::Aretrieve | Self::Alist => ClientKind::Asynchronous,
        }
    }
}

pub(crate) async fn call<Provider: ReadProvider>(
    which: Which,
    provider: &Provider,
    fixture: &Fixture,
    request_timeout_seconds: f64,
) -> Result<Json, VectorStoreReadFailure> {
    call_with(which, provider, fixture, None, request_timeout_seconds).await
}

pub(crate) async fn call_with_client(
    which: Which,
    spy: &Spy,
    fixture: &Fixture,
    supplied: Option<SuppliedClient<'_, FakeClient>>,
    request_timeout_seconds: f64,
) -> Result<Json, VectorStoreReadFailure> {
    call_with(which, spy, fixture, supplied, request_timeout_seconds).await
}

pub(crate) async fn call_with<Provider: ReadProvider>(
    which: Which,
    provider: &Provider,
    fixture: &Fixture,
    supplied: Option<SuppliedClient<'_, Provider::Client>>,
    request_timeout_seconds: f64,
) -> Result<Json, VectorStoreReadFailure> {
    let deps = VectorStoreReadDeps {
        configs: &fixture.configs,
        logging: &fixture.logging,
        redactor: &fixture.redactor,
        supplied,
        request_timeout_seconds,
    };
    let reads = VectorStoreReads { provider, deps };
    match which {
        Which::Retrieve => reads.retrieve(&fixture.request()).await,
        Which::Aretrieve => reads.aretrieve(&fixture.request()).await,
        Which::List => reads.list(&fixture.list_request()).await,
        Which::Alist => reads.alist(&fixture.list_request()).await,
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
        .expect("the read answers");
    spy.seen().expect("the service saw the read")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Query {
    pub(crate) after: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<String>,
}

// what the service saw: the call's fields and the logging state the read left behind
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub(crate) operation: Operation,
    pub(crate) mode: ClientKind,
    pub(crate) provider: ProviderId,
    pub(crate) vector_store_id: Option<String>,
    pub(crate) query: Query,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    pub(crate) timeout: Duration,
    pub(crate) client: Option<(ClientKind, String)>,
    pub(crate) model: String,
    pub(crate) call_type: String,
    pub(crate) optional: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) details: IndexMap<String, Json>,
    pub(crate) provider_name: Option<String>,
}

fn observe(call: &ReadCall<'_>) -> Recorded {
    let state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
    Recorded {
        operation: call.operation,
        mode: call.mode,
        provider: call.provider,
        vector_store_id: call.vector_store_id.map(ToOwned::to_owned),
        query: Query {
            after: call.query.after.map(ToOwned::to_owned),
            before: call.query.before.map(ToOwned::to_owned),
            limit: call.query.limit,
            order: call.query.order.map(ToOwned::to_owned),
        },
        api_base: call.params.api_base.clone(),
        api_key: call
            .params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        extra_headers: call.extra_headers.cloned(),
        extra_body: call.extra_body.cloned(),
        timeout: call.timeout,
        client: call
            .client
            .map(|supplied| (supplied.kind, supplied.transport.name.to_owned())),
        model: state.model.clone(),
        call_type: state.call_type.clone(),
        optional: state.optional_params.clone(),
        litellm_params: state.litellm_params.clone(),
        details: state.model_call_details.clone(),
        provider_name: state.custom_llm_provider.clone(),
    }
}

pub(crate) struct FakeClient {
    pub(crate) name: &'static str,
}

enum Reply {
    Value(Json),
    Provider { status: u16, message: String },
}

// The provider's read service: a recorder in tests, B2485 behind the production adapter
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

    // how many reads reached the service: a stray second delegation shows up here, not only in the last Recorded
    pub(crate) fn call_count(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    fn answer(&self) -> Result<Json, CreateReadError> {
        let reply = self.reply.lock().unwrap_or_else(PoisonError::into_inner);
        match &*reply {
            Reply::Value(value) => Ok(value.clone()),
            Reply::Provider { status, message } => Err(CreateReadError::Write(
                WriteError::Provider(BaseLlmException {
                    status_code: *status,
                    message: message.clone(),
                    headers: HeaderMap::new(),
                    status_code_is_synthesized: false,
                }),
            )),
        }
    }
}

impl ReadProvider for Spy {
    type Client = FakeClient;

    fn read(
        &self,
        call: &ReadCall<'_, FakeClient>,
    ) -> impl Future<Output = Result<Json, CreateReadError>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(observe(call));
        ready(self.answer())
    }
}

pub(crate) fn supplied<'client, Client>(
    kind: ClientKind,
    transport: &'client Client,
) -> SuppliedClient<'client, Client> {
    SuppliedClient { kind, transport }
}
