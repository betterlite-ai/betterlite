use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::response_resources::request::ListInputItemsQuery;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::target;

use crate::api::responses_read_http::config::{
    NoWorkloadIdentity, OpenAiReadConfig, ReadConfig, ReadEnvironment, ReadParams,
};
use crate::api::responses_read_http::error::ReadError;
use crate::api::responses_read_http::service::{
    ClientKind, InputItemsCall, LogContext, PreparedRead, ReadCall, ReadDeps, ReadRequest,
    ReadTransport, RetrieveCall, prepare,
};
use crate::api::vector_store_http_write::error::ClientError;
use crate::api::vector_store_http_write::service::ClientSource;

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

pub(crate) fn extra_body(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the extra body fixture is valid")
}

pub(crate) fn caller_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn params(api_base: &str, api_key: &str) -> ReadParams {
    ReadParams {
        api_base: Some(api_base.to_owned()),
        api_key: Some(SecretString::from(api_key)),
        ssl_verify: None,
    }
}

pub(crate) fn openai_config(environment: ReadEnvironment) -> OpenAiReadConfig<NoWorkloadIdentity> {
    OpenAiReadConfig::new(LiteLlmGlobals::default(), environment, NoWorkloadIdentity)
}

pub(crate) fn retrieve_call(api: &ReadParams) -> ReadCall<'_> {
    ReadCall::Retrieve(RetrieveCall {
        target: target("resp_a", api, None, None),
        extra_body: None,
    })
}

pub(crate) fn input_items_call<'call>(
    api: &'call ReadParams,
    query: &'call ListInputItemsQuery,
) -> ReadCall<'call> {
    ReadCall::InputItems(InputItemsCall {
        target: target("resp_a", api, None, None),
        query,
    })
}

pub(crate) fn header<'headers>(
    headers: &'headers HttpHeaders,
    name: &str,
) -> Option<&'headers str> {
    headers.get(name)?.to_str().ok()
}

#[derive(Clone, Default)]
pub(crate) struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(crate) fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(crate) fn new(call_type: &str) -> Self {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"responses","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        ))
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            state,
            now,
        }
    }

    pub(crate) fn prepare<'deps, C, S>(
        &mut self,
        deps: &'deps ReadDeps<'deps, C, S>,
        mode: ClientKind,
        call: &ReadCall<'_>,
        events: &Events,
    ) -> Result<PreparedRead<'deps, C, S>, ReadError>
    where
        C: ReadConfig,
        S: ClientSource,
    {
        let logger = |_details: &Json| -> Result<(), PrePostError> {
            events.push("log");
            Ok(())
        };
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: Some(&logger),
            now: self.now,
        };
        prepare(
            deps,
            mode,
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
    }

    pub(crate) fn logged(&self, name: &str) -> Option<&Json> {
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        details.get(name)
    }

    pub(crate) fn input(&self) -> Option<&Json> {
        self.state.model_call_details.get("input")
    }
}

#[derive(Clone)]
pub(crate) struct Reply {
    status: u16,
    headers: HttpHeaders,
    body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.insert(
            name,
            http::HeaderValue::from_str(value).expect("the header value"),
        );
        self
    }
}

#[derive(Clone)]
pub(crate) struct Wire {
    reply: Reply,
    failure: Arc<Mutex<Option<TransportError>>>,
    requests: Arc<Mutex<Vec<ReadRequest>>>,
    events: Events,
}

impl Wire {
    pub(crate) fn replying(reply: Reply, events: &Events) -> Self {
        Self {
            reply,
            failure: Arc::default(),
            requests: Arc::default(),
            events: events.clone(),
        }
    }

    pub(crate) fn requests(&self) -> MutexGuard<'_, Vec<ReadRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ReadTransport for Wire {
    fn send(
        &self,
        request: ReadRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.events.push("send");
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if let Some(error) = failure {
            return ready(Err(error));
        }
        ready(Ok(UpstreamResponse {
            status: self.reply.status,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        }))
    }
}

pub(crate) struct Source {
    wire: Wire,
    calls: Arc<Mutex<Vec<Option<bool>>>>,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self {
            wire,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Option<bool>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self, ssl_verify: Option<bool>) -> Result<Wire, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify);
        Ok(self.wire.clone())
    }
}

pub(crate) const IMPATIENT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(100),
    ..TransportLimits::DEFAULT
};

pub(crate) fn proxied_client(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::build(base_builder().http1_only(), &policy, limits)
        .expect("the proxied client builds")
}

pub(crate) struct ProxiedSource {
    client: ProxiedClient,
}

impl ProxiedSource {
    pub(crate) fn new(limits: TransportLimits) -> Self {
        Self {
            client: proxied_client(limits),
        }
    }
}

impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

#[derive(Clone, Default)]
pub(crate) struct Capture(Arc<Mutex<Vec<String>>>);

impl Capture {
    pub(crate) fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(visitor.message);
    }
}

#[derive(Default)]
struct Visitor {
    message: String,
}

impl Visit for Visitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        }
    }
}

pub(crate) fn capture() -> (Capture, DefaultGuard) {
    let captured = Capture::default();
    let guard = set_default(registry().with(captured.clone()));
    (captured, guard)
}
