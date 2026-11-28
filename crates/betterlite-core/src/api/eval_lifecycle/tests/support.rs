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
use indexmap::IndexMap;

use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_lifecycle::error::{EvalLifecycleError, EvalLifecycleFailure};
use crate::api::eval_lifecycle::{
    EvalLifecycleCall, EvalLifecycleConfigLookup, EvalLifecycleDeps, EvalLifecycles,
};

pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const DELETED: &str = r#"{"eval_id":"eval_1","object":"eval.deleted","deleted":true}"#;
pub(crate) const CANCELLED: &str = r#"{"id":"eval_1","object":"eval","status":"cancelled"}"#;

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

type Logger = Box<dyn Fn(&Json) -> Result<(), PrePostError> + Send + Sync>;

// The fixture's logging object: one redactor, one sink, and a logger function that keeps what the pre-call recorded
struct LogEnv {
    redactor: Redactor,
    sink: Sink,
    logger: Logger,
    snapshot: Arc<Mutex<Option<Json>>>,
    now: DateTime<FixedOffset>,
}

impl LogEnv {
    fn new() -> Self {
        let snapshot: Arc<Mutex<Option<Json>>> = Arc::default();
        let kept = Arc::clone(&snapshot);
        Self {
            redactor: redactor(),
            sink: Sink,
            logger: Box::new(move |details: &Json| {
                *kept.lock().unwrap_or_else(PoisonError::into_inner) = Some(details.clone());
                Ok(())
            }),
            snapshot,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
        }
    }

    fn pre_post(&self) -> PrePostEnv<'_> {
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
}

pub(crate) struct Fixture {
    pub(crate) state: LoggingState,
    env: LogEnv,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"eval","call_type":"eval","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .expect("the logging fixture is valid");
        let redactor = redactor();
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
            env: LogEnv::new(),
        }
    }

    // what the pre-call handed the callback, which is the logging state as it stood before the send
    pub(crate) fn snapshot(&self) -> Option<Json> {
        self.env
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn detail(&self, name: &str) -> Option<&Json> {
        self.state.model_call_details.get(name)
    }

    pub(crate) fn logged(&self, name: &str) -> Option<&Json> {
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        details.get(name)
    }
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")
}

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

pub(crate) fn url(serving: &Serving) -> String {
    format!("http://{}", serving.address)
}

pub(crate) fn assert_line(request: &Request, method: &str, path: &str) {
    assert_eq!(request.line, format!("{method} {path} HTTP/1.1"));
}

pub(crate) fn head_value<'head>(head: &'head str, name: &str) -> Option<&'head str> {
    head.lines().find_map(|line| {
        let (found, value) = line.split_once(':')?;
        found.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

// A call that stopped before the send leaves the fixture with no request to read
pub(crate) fn assert_no_request(serving: Serving) {
    let outcome = serving.finish();
    assert!(matches!(outcome, Err(FixtureError::NoRequest)));
}

// the validated headers reach the transform, and the handler sends what the transform returned
pub(crate) fn assert_forwarded_headers(seen: &Request, asked: &Asked) {
    assert_eq!(
        head_value(&seen.head, "authorization"),
        Some("Bearer sk-test")
    );
    assert_eq!(head_value(&seen.head, "x-validated"), Some("yes"));
    assert_eq!(
        head_value(&seen.head, "x-transformed"),
        Some("yes"),
        "the handler sends the headers the transform returned"
    );
    assert_eq!(head_value(&seen.head, "x-extra"), Some("one"));
    assert_eq!(
        asked.headers.get("x-validated").map(String::as_str),
        Some("yes")
    );
    assert_eq!(
        asked.headers.get("x-extra").map(String::as_str),
        Some("one")
    );
    let transformed = asked.headers.get("x-transformed").map(String::as_str);
    assert_eq!(
        transformed, None,
        "the transform sees the validated headers"
    );
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture is valid")
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn kwargs(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

// the keywords a real call carries: the key B523's validate_environment reads and the base the fixture answers on
pub(crate) fn openai_kwargs(base: &str) -> IndexMap<String, Json> {
    kwargs(&[("api_key", text("sk-test")), ("api_base", text(base))])
}

pub(crate) const fn call<'call>(
    eval_id: &'call str,
    kwargs: &'call IndexMap<String, Json>,
) -> EvalLifecycleCall<'call> {
    EvalLifecycleCall {
        eval_id,
        extra_headers: None,
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
        kwargs,
    }
}

// B523's config as B024 resolves it for "openai", and None for every other name
pub(crate) struct RealConfigs;

impl EvalLifecycleConfigLookup for RealConfigs {
    type Config = OpenAiEvalsConfig;

    fn eval_config(&self, provider: &str) -> Option<OpenAiEvalsConfig> {
        match provider {
            "openai" => Some(OpenAiEvalsConfig::default()),
            _ => None,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Recorder(Arc<Mutex<Recorded>>);

#[derive(Debug, Default)]
struct Recorded {
    providers: Vec<String>,
    calls: Vec<Asked>,
}

#[derive(Debug, Clone)]
pub(crate) struct Asked {
    pub(crate) eval_id: String,
    pub(crate) api_base: String,
    pub(crate) headers: Headers,
    pub(crate) params: Object,
    pub(crate) cancel: bool,
}

impl Asked {
    fn of(eval_id: &str, api_base: &str, context: &EvalsRequestContext<'_>, cancel: bool) -> Self {
        Self {
            eval_id: eval_id.to_owned(),
            api_base: api_base.to_owned(),
            headers: context.headers.clone(),
            params: context.litellm_params.clone(),
            cancel,
        }
    }
}

impl Recorder {
    fn provider(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .providers
            .push(name.to_owned());
    }

    fn transform(&self, asked: Asked) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .push(asked);
    }

    pub(crate) fn providers(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .providers
            .clone()
    }

    pub(crate) fn calls(&self) -> Vec<Asked> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .clone()
    }

    pub(crate) fn only_call(&self) -> Asked {
        let calls = self.calls();
        assert_eq!(calls.len(), 1, "the transform ran once");
        calls.into_iter().next().expect("one recorded call")
    }
}

pub(crate) struct RecordingLookup {
    recorder: Recorder,
    fixture_base: String,
}

impl RecordingLookup {
    pub(crate) fn new(fixture_base: String) -> Self {
        Self {
            recorder: Recorder::default(),
            fixture_base,
        }
    }

    pub(crate) fn recorder(&self) -> &Recorder {
        &self.recorder
    }
}

impl EvalLifecycleConfigLookup for RecordingLookup {
    type Config = RecordingConfig;

    fn eval_config(&self, provider: &str) -> Option<RecordingConfig> {
        self.recorder.provider(provider);
        match provider {
            "openai" => Some(RecordingConfig {
                inner: OpenAiEvalsConfig::default(),
                recorder: self.recorder.clone(),
                fixture_base: self.fixture_base.clone(),
            }),
            _ => None,
        }
    }
}

// The real OpenAI evals config behind a recorder: validate_environment marks its answer, and every transform keeps what it was handed
pub(crate) struct RecordingConfig {
    inner: OpenAiEvalsConfig,
    recorder: Recorder,
    fixture_base: String,
}

fn marked(headers: &Headers) -> Headers {
    let mut marked = headers.clone();
    marked.insert("x-transformed".to_owned(), "yes".to_owned());
    marked
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

impl BaseEvalsAPIConfig for RecordingConfig {
    type Error = EvalsError;

    // B523's answer plus a header only validate_environment can add
    fn validate_environment(
        &self,
        headers: Headers,
        litellm_params: Option<&Object>,
    ) -> Result<Headers, EvalsError> {
        let mut validated = self.inner.validate_environment(headers, litellm_params)?;
        validated.insert("x-validated".to_owned(), "yes".to_owned());
        Ok(validated)
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        endpoint: &str,
        eval_id: Option<&str>,
    ) -> Result<String, EvalsError> {
        self.inner.get_complete_url(api_base, endpoint, eval_id)
    }

    #[rustfmt::skip]
    fn transform_delete_eval_request(&self, eval_id: &str, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers), EvalsError> {
        self.recorder.transform(Asked::of(eval_id, api_base, context, false));
        Ok((format!("{}/v1/evals/{eval_id}", self.fixture_base), marked(context.headers)))
    }

    #[rustfmt::skip]
    fn transform_cancel_eval_request(&self, eval_id: &str, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers, Object), EvalsError> {
        self.recorder.transform(Asked::of(eval_id, api_base, context, true));
        // a body the transport must never send
        let body = object(r#"{"dropped":"not on the wire"}"#);
        Ok((format!("{}/v1/evals/{eval_id}/cancel", self.fixture_base), marked(context.headers), body))
    }

    delegated_requests! {
        transform_create_eval_request(create_request: &CreateEvalRequest) -> Object;
        transform_list_evals_request(list_params: &ListEvalsParams) -> (String, Object);
        transform_get_eval_request(eval_id: &str, api_base: &str) -> (String, Headers);
        transform_update_eval_request(eval_id: &str, update_request: &UpdateEvalRequest, api_base: &str) -> (String, Headers, Object);
        transform_create_run_request(eval_id: &str, create_request: &CreateRunRequest) -> (String, Object);
        transform_list_runs_request(eval_id: &str, list_params: &ListRunsParams) -> (String, Object);
        transform_get_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers);
        transform_cancel_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers, Object);
        transform_delete_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers, Object);
    }

    delegated_responses! {
        transform_create_eval_response -> Eval;
        transform_list_evals_response -> ListEvalsResponse;
        transform_get_eval_response -> Eval;
        transform_update_eval_response -> Eval;
        transform_delete_eval_response -> DeleteEvalResponse;
        transform_cancel_eval_response -> CancelEvalResponse;
        transform_create_run_response -> Run;
        transform_list_runs_response -> ListRunsResponse;
        transform_get_run_response -> Run;
        transform_cancel_run_response -> CancelRunResponse;
        transform_delete_run_response -> RunDeleteResponse;
    }
}

pub(crate) struct Rig<Configs> {
    pub(crate) fixture: Fixture,
    pub(crate) configs: Configs,
    pub(crate) client: ProxiedClient,
    pub(crate) redactor: Redactor,
    pub(crate) request_timeout: Duration,
}

impl<Configs> Rig<Configs> {
    pub(crate) fn new(
        configs: Configs,
        limits: TransportLimits,
        request_timeout: Duration,
    ) -> Self {
        Self {
            fixture: Fixture::new(),
            configs,
            client: proxied_client(limits),
            redactor: redactor(),
            request_timeout,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Operation {
    Delete,
    AsyncDelete,
    Cancel,
    AsyncCancel,
}

#[derive(Debug)]
pub(crate) enum Answer {
    Deleted(Box<DeleteEvalResponse>),
    Cancelled(Box<CancelEvalResponse>),
}

// One call: the fixture's logging state, the deps and the operation the case names
#[rustfmt::skip]
pub(crate) async fn run<Configs>(rig: &mut Rig<Configs>, operation: Operation, call: &EvalLifecycleCall<'_>) -> Result<Answer, EvalLifecycleFailure>
where
    Configs: EvalLifecycleConfigLookup,
{
    let env = rig.fixture.env.pre_post();
    let deps = EvalLifecycleDeps { configs: &rig.configs, client: &rig.client, redactor: &rig.redactor, request_timeout: rig.request_timeout };
    let mut log = LogContext { state: &mut rig.fixture.state, env: &env };
    let mut operations = EvalLifecycles { deps: &deps, log: &mut log };
    match operation {
        Operation::Delete => operations.delete_eval(call).await.map(|reply| Answer::Deleted(Box::new(reply))),
        Operation::AsyncDelete => operations.adelete_eval(call).await.map(|reply| Answer::Deleted(Box::new(reply))),
        Operation::Cancel => operations.cancel_eval(call).await.map(|reply| Answer::Cancelled(Box::new(reply))),
        Operation::AsyncCancel => operations.acancel_eval(call).await.map(|reply| Answer::Cancelled(Box::new(reply))),
    }
}

pub(crate) fn deleted(answer: Answer) -> DeleteEvalResponse {
    let Answer::Deleted(reply) = answer else {
        panic!("the call answers with a delete response");
    };
    *reply
}

pub(crate) fn cancelled(answer: Answer) -> CancelEvalResponse {
    let Answer::Cancelled(reply) = answer else {
        panic!("the call answers with a cancel response");
    };
    *reply
}

// The provider error a handler failure carries, which exception_type maps with its status and text
pub(crate) fn provider(failure: &EvalLifecycleFailure) -> &ProviderError {
    let EvalLifecycleError::Handler(EvalHttpError::Api(EvalsError::Provider { provider })) =
        &failure.error
    else {
        panic!(
            "the call answers with the provider's error, not {}",
            failure.error
        );
    };
    provider
}
