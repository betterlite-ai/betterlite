use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http_fixture::error::FixtureError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::request::Request;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_logging::logging_pre_post::context::LogContext;
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
use betterlite_openai::error::ProviderError;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;
use betterlite_openai::evals::transformation::{
    BaseEvalsAPIConfig, EvalsHttpResponse, EvalsRequestContext,
};
use betterlite_openai::evals::types::{
    CancelEvalResponse, CancelRunResponse, CreateEvalRequest, CreateRunRequest, DeleteEvalResponse,
    Eval, ListEvalsParams, ListEvalsResponse, ListRunsParams, ListRunsResponse, Run,
    RunDeleteResponse, UpdateEvalRequest,
};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};

use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_http::exchange::{EvalConnection, EvalExchange, EvalReply, eval_exchange};
use crate::api::eval_http::handlers::{
    cancel_eval_handler, create_eval_handler, delete_eval_handler, get_eval_handler,
    list_evals_handler, update_eval_handler,
};

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

pub(crate) type Logger = Box<dyn Fn(&Json) -> Result<(), PrePostError> + Send + Sync>;

// The fixture's logging object: one redactor, one sink, and a logger function that keeps what the call recorded
pub(crate) struct LogEnv {
    redactor: Redactor,
    sink: Sink,
    logger: Logger,
    snapshot: Arc<Mutex<Option<Json>>>,
    now: DateTime<FixedOffset>,
}

impl LogEnv {
    fn new(events: &Events) -> Self {
        let recorded = events.clone();
        let snapshot: Arc<Mutex<Option<Json>>> = Arc::default();
        let kept = Arc::clone(&snapshot);
        Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
            sink: Sink,
            logger: Box::new(move |details: &Json| {
                recorded.push("log");
                *kept.lock().unwrap_or_else(PoisonError::into_inner) = Some(details.clone());
                Ok(())
            }),
            snapshot,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
        }
    }

    pub(crate) fn pre_post(&self) -> PrePostEnv<'_> {
        PrePostEnv {
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
            logger_fn: Some(&*self.logger),
            now: self.now,
        }
    }

    pub(crate) fn snapshot(&self) -> Option<Json> {
        self.snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct Fixture {
    pub(crate) state: LoggingState,
    pub(crate) env: LogEnv,
}

impl Fixture {
    pub(crate) fn new(call_type: &str, events: &Events) -> Self {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"eval","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        ))
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
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
            state,
            env: LogEnv::new(events),
        }
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

    pub(crate) fn snapshot(&self) -> Option<Json> {
        self.env.snapshot()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Seen {
    pub(crate) transform: &'static str,
    pub(crate) status_code: u16,
    pub(crate) headers: Headers,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Asked {
    pub(crate) message: String,
    pub(crate) status_code: u16,
}

// The real OpenAI evals config behind a recorder: the six response transforms and get_error_class keep what they were handed
pub(crate) struct RecordingConfig {
    inner: OpenAiEvalsConfig,
    events: Events,
    seen: Arc<Mutex<Vec<Seen>>>,
    asked: Arc<Mutex<Vec<Asked>>>,
}

macro_rules! delegated_requests {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $out:ty;)+) => {
        $(
            fn $name(
                &self,
                $($arg: $ty,)*
                context: &EvalsRequestContext<'_>,
            ) -> Result<$out, EvalsError> {
                self.inner.$name($($arg,)* context)
            }
        )+
    };
}

macro_rules! delegated_responses {
    ($($name:ident -> $out:ty;)+) => {
        $(
            fn $name(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<$out, EvalsError> {
                self.inner.$name(raw_response)
            }
        )+
    };
}

impl RecordingConfig {
    pub(crate) fn new(events: &Events) -> Self {
        Self {
            inner: OpenAiEvalsConfig::default(),
            events: events.clone(),
            seen: Arc::default(),
            asked: Arc::default(),
        }
    }

    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn asked(&self) -> Vec<Asked> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, transform: &'static str, reply: &EvalsHttpResponse<'_>) {
        self.events.push(transform);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                transform,
                status_code: reply.status_code,
                headers: reply.headers.clone(),
                text: reply.text.to_owned(),
            });
    }
}

impl BaseEvalsAPIConfig for RecordingConfig {
    type Error = EvalsError;

    fn validate_environment(
        &self,
        headers: Headers,
        litellm_params: Option<&Object>,
    ) -> Result<Headers, EvalsError> {
        self.inner.validate_environment(headers, litellm_params)
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        endpoint: &str,
        eval_id: Option<&str>,
    ) -> Result<String, EvalsError> {
        self.inner.get_complete_url(api_base, endpoint, eval_id)
    }

    delegated_requests! {
        transform_create_eval_request(create_request: &CreateEvalRequest) -> Object;
        transform_list_evals_request(list_params: &ListEvalsParams) -> (String, Object);
        transform_get_eval_request(eval_id: &str, api_base: &str) -> (String, Headers);
        transform_update_eval_request(eval_id: &str, update_request: &UpdateEvalRequest, api_base: &str) -> (String, Headers, Object);
        transform_delete_eval_request(eval_id: &str, api_base: &str) -> (String, Headers);
        transform_create_run_request(eval_id: &str, create_request: &CreateRunRequest) -> (String, Object);
        transform_list_runs_request(eval_id: &str, list_params: &ListRunsParams) -> (String, Object);
        transform_get_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers);
        transform_cancel_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers, Object);
        transform_delete_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers, Object);
    }

    fn transform_create_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, EvalsError> {
        self.record("create_response", raw_response);
        self.inner.transform_create_eval_response(raw_response)
    }

    fn transform_list_evals_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<ListEvalsResponse, EvalsError> {
        self.record("list_response", raw_response);
        self.inner.transform_list_evals_response(raw_response)
    }

    fn transform_get_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, EvalsError> {
        self.record("get_response", raw_response);
        self.inner.transform_get_eval_response(raw_response)
    }

    fn transform_update_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, EvalsError> {
        self.record("update_response", raw_response);
        self.inner.transform_update_eval_response(raw_response)
    }

    fn transform_delete_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<DeleteEvalResponse, EvalsError> {
        self.record("delete_response", raw_response);
        self.inner.transform_delete_eval_response(raw_response)
    }

    fn transform_cancel_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<CancelEvalResponse, EvalsError> {
        self.record("cancel_response", raw_response);
        self.inner.transform_cancel_eval_response(raw_response)
    }

    // A body the transform would build if a caller asked it to; the cancel handler must post {} without asking
    fn transform_cancel_eval_request(
        &self,
        _eval_id: &str,
        _api_base: &str,
        _context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers, Object), EvalsError> {
        self.events.push("cancel_request");
        Ok((
            String::new(),
            Headers::new(),
            object(r#"{"dropped":"not on the wire"}"#),
        ))
    }

    delegated_responses! {
        transform_create_run_response -> Run;
        transform_list_runs_response -> ListRunsResponse;
        transform_get_run_response -> Run;
        transform_cancel_run_response -> CancelRunResponse;
        transform_delete_run_response -> RunDeleteResponse;
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> EvalsError {
        self.events.push("error_class");
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Asked {
                message: error_message.to_owned(),
                status_code,
            });
        self.inner
            .get_error_class(error_message, status_code, headers)
    }
}

pub(crate) struct Bench {
    pub(crate) events: Events,
    pub(crate) fixture: Fixture,
    pub(crate) config: RecordingConfig,
    pub(crate) client: ProxiedClient,
    pub(crate) url: String,
    pub(crate) headers: Headers,
}

impl Bench {
    pub(crate) fn new(url: String, limits: TransportLimits) -> Self {
        let events = Events::new();
        let fixture = Fixture::new("create_eval", &events);
        let config = RecordingConfig::new(&events);
        Self {
            events,
            fixture,
            config,
            client: proxied_client(limits),
            url,
            headers: headers(&[("x-test", "yes")]),
        }
    }
}

pub(crate) const IMPATIENT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(100),
    ..TransportLimits::DEFAULT
};

fn proxied_client(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::build(base_builder().http1_only(), &policy, limits)
        .expect("the proxied client builds")
}

pub(crate) fn json_server(status: &str, body: &str, pause: Duration) -> Serving {
    serve(
        reply(
            status,
            &[("content-type", "application/json"), ("x-upstream", "yes")],
            body,
        ),
        pause,
    )
    .expect("the fixture serves")
}

pub(crate) fn url(serving: &Serving, path: &str) -> String {
    format!("http://{}{path}", serving.address)
}

// A call that stopped before the send leaves the fixture with no request to read
pub(crate) fn assert_no_request(serving: Serving) {
    let outcome = serving.finish();
    assert!(matches!(outcome, Err(FixtureError::NoRequest)));
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture is valid")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the JSON fixture is valid")
}

pub(crate) fn body_json(request: &Request) -> Json {
    serde_json::from_str(&request.body).expect("the request body is JSON")
}

pub(crate) fn assert_line(request: &Request, method: &str, path: &str) {
    assert_eq!(request.line, format!("{method} {path} HTTP/1.1"));
}

// What one call answered, so a case can match the response the transform built without naming its handler
#[derive(Debug)]
pub(crate) enum Answer {
    Eval(Box<Eval>),
    Page(Box<ListEvalsResponse>),
    Deleted(Box<DeleteEvalResponse>),
    Cancelled(Box<CancelEvalResponse>),
    Reply(EvalReply),
}

// The operation one case runs, with the payload it carries; the exchange case hands eval_exchange itself a whole request
pub(crate) enum Call<'call> {
    Create {
        timeout: Option<Duration>,
        body: Object,
    },
    List {
        query: Object,
    },
    Get,
    Update {
        timeout: Option<Duration>,
        body: Object,
    },
    Delete {
        timeout: Option<Duration>,
    },
    Cancel {
        timeout: Option<Duration>,
    },
    Exchange(&'call EvalExchange<'call>),
}

// One case's call: the fixture's logging state, config, client, URL and headers set up once, then the handler the case names
#[rustfmt::skip]
pub(crate) async fn run(bench: &mut Bench, call: Call<'_>) -> Result<Answer, EvalHttpError> {
    let env = bench.fixture.env.pre_post();
    let connection = EvalConnection { config: &bench.config, client: &bench.client, url: &bench.url, headers: &bench.headers };
    let mut log = LogContext { state: &mut bench.fixture.state, env: &env };
    match call {
        Call::Create { timeout, body } => Ok(Answer::Eval(Box::new(create_eval_handler(&connection, &mut log, timeout, &body).await?))),
        Call::List { query } => Ok(Answer::Page(Box::new(list_evals_handler(&connection, &mut log, &query).await?))),
        Call::Get => Ok(Answer::Eval(Box::new(get_eval_handler(&connection, &mut log).await?))),
        Call::Update { timeout, body } => Ok(Answer::Eval(Box::new(update_eval_handler(&connection, &mut log, timeout, &body).await?))),
        Call::Delete { timeout } => Ok(Answer::Deleted(Box::new(delete_eval_handler(&connection, &mut log, timeout).await?))),
        Call::Cancel { timeout } => Ok(Answer::Cancelled(Box::new(cancel_eval_handler(&connection, &mut log, timeout).await?))),
        Call::Exchange(exchange) => Ok(Answer::Reply(eval_exchange(&connection, exchange, &mut log).await?)),
    }
}

pub(crate) fn provider(error: &EvalHttpError) -> &ProviderError {
    let EvalHttpError::Api(EvalsError::Provider { provider }) = error else {
        panic!("the handler answers with the provider's error, not {error}");
    };
    provider
}
