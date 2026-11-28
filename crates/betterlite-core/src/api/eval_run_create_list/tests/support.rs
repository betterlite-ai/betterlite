use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::Duration;

use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
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
use futures_executor::block_on;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_http::exchange::EvalConfig;
use crate::api::eval_run_create_list::args::{CreateRunArgs, ListRunsArgs};
use crate::api::eval_run_create_list::error::{EvalRunCallError, EvalRunFailure};
use crate::api::eval_run_create_list::providers::{
    CreateRunCall, EvalRunProviders, ListRunsCall, PendingCreateRun, PendingListRuns, RunMode,
    RunTransport,
};
use crate::api::eval_run_create_list::{CreateRunWork, EvalRunCreateList, ListRunsWork};

// the request timeout a deployment injects, REQUEST_TIMEOUT's 100 minutes
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_mins(100);

// what the transforms recorded: the operation, the eval id and the headers the context carried
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TransformCall {
    pub(crate) operation: &'static str,
    pub(crate) eval_id: String,
    pub(crate) headers: Headers,
}

// the real OpenAI evals config behind a recorder, so a case can read the context the transform was handed
#[derive(Default)]
pub(crate) struct RecordingConfig {
    inner: OpenAiEvalsConfig,
    seen: Arc<Mutex<Vec<TransformCall>>>,
}

impl RecordingConfig {
    pub(crate) fn with_key(key: &str) -> Self {
        Self {
            inner: OpenAiEvalsConfig {
                api_key: Some(SecretString::from(key)),
                openai_key: None,
                env_api_key: None,
            },
            seen: Arc::default(),
        }
    }

    pub(crate) fn without_key() -> Self {
        Self::default()
    }

    pub(crate) fn transforms(&self) -> Vec<TransformCall> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, operation: &'static str, eval_id: &str, context: &EvalsRequestContext<'_>) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(TransformCall {
                operation,
                eval_id: eval_id.to_owned(),
                headers: context.headers.clone(),
            });
    }
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

macro_rules! delegated_replies {
    ($($name:ident -> $out:ty;)+) => {
        $(
            fn $name(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<$out, EvalsError> {
                self.inner.$name(raw_response)
            }
        )+
    };
}

// the two run transforms record the context they were handed, then pass the same call to the real config
macro_rules! recorded_transform {
    ($method:ident, $operation:expr, $request:ident, $request_type:ty) => {
        fn $method(
            &self,
            eval_id: &str,
            $request: &$request_type,
            context: &EvalsRequestContext<'_>,
        ) -> Result<(String, Object), EvalsError> {
            self.record($operation, eval_id, context);
            self.inner.$method(eval_id, $request, context)
        }
    };
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
        transform_cancel_eval_request(eval_id: &str, api_base: &str) -> (String, Headers, Object);
        transform_get_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers);
        transform_cancel_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers, Object);
        transform_delete_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers, Object);
    }

    recorded_transform!(
        transform_create_run_request,
        "create_run",
        create_request,
        CreateRunRequest
    );

    recorded_transform!(
        transform_list_runs_request,
        "list_runs",
        list_params,
        ListRunsParams
    );

    delegated_replies! {
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

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> EvalsError {
        self.inner
            .get_error_class(error_message, status_code, headers)
    }
}

impl EvalRunProviders for RecordingConfig {
    fn evals_config(&self, tag: EvalsConfigTag) -> Option<EvalConfig<'_>> {
        match tag {
            EvalsConfigTag::OpenAI => Some(self),
        }
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// the environment a pre-call would read; these handlers never pre-call, so only the redactor and clock are real
pub(crate) struct Env {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl Env {
    fn new() -> Self {
        Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
            sink: Sink,
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
            logger_fn: None,
            now: self.now,
        }
    }
}

pub(crate) struct Fixture {
    pub(crate) state: LoggingState,
    pub(crate) env: Env,
}

impl Fixture {
    pub(crate) fn new(call_type: &str) -> Self {
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
            env: Env::new(),
        }
    }

    pub(crate) fn detail(&self, name: &str) -> Option<&Json> {
        self.state.model_call_details.get(name)
    }
}

// what the caller struct needs from a deployment: the selector, the handlers, B185's redactor and the request timeout
pub(crate) fn caller(
    config: RecordingConfig,
    recorder: Recorder,
) -> EvalRunCreateList<RecordingConfig, Recorder> {
    EvalRunCreateList {
        providers: config,
        transport: recorder,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
        request_timeout: REQUEST_TIMEOUT,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Provider,
    Local,
    // answers on the second poll: a caller that polls once never sees it, one that keeps awaiting does
    Deferred,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SeenCreate {
    pub(crate) provider: String,
    pub(crate) url: String,
    pub(crate) headers: Headers,
    pub(crate) body: Object,
    pub(crate) timeout: Duration,
    pub(crate) mode: RunMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SeenList {
    pub(crate) provider: String,
    pub(crate) url: String,
    pub(crate) headers: Headers,
    pub(crate) query: Object,
    pub(crate) timeout: Duration,
    pub(crate) mode: RunMode,
}

// B9371's two handlers behind a recorder: each call is kept, and the answer is the fixture or the failure a case asks for
#[derive(Default)]
pub(crate) struct Recorder {
    creates: Arc<Mutex<Vec<SeenCreate>>>,
    lists: Arc<Mutex<Vec<SeenList>>>,
    failure: Option<Failure>,
}

impl Recorder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn failing(failure: Failure) -> Self {
        Self {
            creates: Arc::default(),
            lists: Arc::default(),
            failure: Some(failure),
        }
    }

    pub(crate) fn creates(&self) -> Vec<SeenCreate> {
        self.creates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn lists(&self) -> Vec<SeenList> {
        self.lists
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// the future a recorded handler hands back, whatever reply type the case asked the handler for
type AnswerFuture<'call, Reply> =
    Pin<Box<dyn Future<Output = Result<Reply, EvalRunCallError>> + Send + 'call>>;

// the answer both handlers give: the fixture, the failure a case asked for, or one that answers on the second poll
fn answer<'call, Reply>(
    failure: Option<Failure>,
    ready: fn() -> Reply,
) -> AnswerFuture<'call, Reply>
where
    Reply: Send + 'call,
{
    Box::pin(async move {
        match failure {
            Some(Failure::Provider) => return Err(provider_failure()),
            Some(Failure::Local) => return Err(local_failure()),
            Some(Failure::Deferred) => {}
            None => return Ok(ready()),
        }
        let mut first = true;
        poll_fn(move |context| {
            if first {
                first = false;
                context.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(Ok(ready()))
            }
        })
        .await
    })
}

// the two handlers differ only in their call, the recorded fields and the fixture, so one template spells them
macro_rules! recorded_transport {
    ($method:ident, $call:ident, $pending:ident, $seen:ident, $slot:ident, $payload:ident, $fixture:path) => {
        fn $method<'call>(
            &'call self,
            _log: LogContext<'call, '_>,
            call: $call<'call>,
        ) -> $pending<'call> {
            self.$slot
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push($seen {
                    provider: call.provider,
                    url: call.url,
                    headers: call.headers,
                    $payload: call.$payload,
                    timeout: call.timeout,
                    mode: call.mode,
                });
            answer(self.failure, $fixture)
        }
    };
}

impl RunTransport for Recorder {
    recorded_transport!(
        create_run,
        CreateRunCall,
        PendingCreateRun,
        SeenCreate,
        creates,
        body,
        run_fixture
    );

    recorded_transport!(
        list_runs,
        ListRunsCall,
        PendingListRuns,
        SeenList,
        lists,
        query,
        runs_fixture
    );
}

fn provider_failure() -> EvalRunCallError {
    EvalRunCallError::Transport(EvalHttpError::Api(EvalsError::Provider {
        provider: Box::new(ProviderError::from(ProviderErrorParts {
            status_code: 429,
            message: "rate limited".to_owned(),
            headers: None,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })),
    }))
}

fn local_failure() -> EvalRunCallError {
    EvalRunCallError::Transport(EvalHttpError::Record {
        source: serde_json::from_str::<Object>("{").expect_err("the fixture text is not JSON"),
    })
}

pub(crate) fn run_fixture() -> Run {
    serde_json::from_str(
        r#"{"id":"run-1","created_at":1,"status":"queued","data_source":{},"eval_id":"eval-1"}"#,
    )
    .expect("the run fixture is valid")
}

pub(crate) fn runs_fixture() -> ListRunsResponse {
    serde_json::from_str(r#"{"data":[{"id":"run-1","created_at":1,"status":"queued","data_source":{},"eval_id":"eval-1"}],"has_more":false}"#)
        .expect("the runs fixture is valid")
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture is valid")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the json fixture is valid")
}

pub(crate) fn kwargs(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the kwargs fixture is valid")
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn header<'call>(headers: &'call Headers, name: &str) -> &'call str {
    headers
        .get(name)
        .map(String::as_str)
        .expect("the header is sent")
}

// a request with every optional value absent, so a case sets only the keyword it exercises
macro_rules! absent_args {
    (create $name:ident, $args:ident) => {
        pub(crate) const fn $name<'call>(
            eval_id: &'call str,
            data_source: &'call Object,
            kwargs: &'call IndexMap<String, Json>,
        ) -> $args<'call> {
            $args {
                eval_id,
                data_source,
                name: None,
                metadata: None,
                extra_headers: None,
                extra_query: None,
                extra_body: None,
                timeout: None,
                custom_llm_provider: None,
                kwargs,
            }
        }
    };
    (list $name:ident, $args:ident) => {
        pub(crate) const fn $name<'call>(
            eval_id: &'call str,
            kwargs: &'call IndexMap<String, Json>,
        ) -> $args<'call> {
            $args {
                eval_id,
                limit: None,
                after: None,
                before: None,
                order: None,
                extra_headers: None,
                extra_query: None,
                timeout: None,
                custom_llm_provider: None,
                kwargs,
            }
        }
    };
}

absent_args!(create create_args, CreateRunArgs);
absent_args!(list list_args, ListRunsArgs);

pub(crate) fn ready(work: CreateRunWork<'_>) -> Run {
    match work {
        CreateRunWork::Ready(run) => *run,
        CreateRunWork::Pending(_pending) => panic!("a sync call answers at once"),
    }
}

pub(crate) fn ready_runs(work: ListRunsWork<'_>) -> ListRunsResponse {
    match work {
        ListRunsWork::Ready(runs) => *runs,
        ListRunsWork::Pending(_pending) => panic!("a sync call answers at once"),
    }
}

pub(crate) fn error<Output>(result: Result<Output, EvalRunFailure>) -> EvalRunFailure {
    match result {
        Ok(_answer) => panic!("the call fails"),
        Err(failure) => failure,
    }
}

// one call over a fresh fixture: the answer, the logging state it filled, and the caller struct it ran through
macro_rules! call_helper {
    (sync $name:ident, $call_type:expr, $request:ident, $reply:ty, $method:ident, $ready:path) => {
        pub(crate) fn $name(
            config: RecordingConfig,
            recorder: Recorder,
            request: $request<'_>,
        ) -> (
            Result<$reply, EvalRunFailure>,
            Fixture,
            EvalRunCreateList<RecordingConfig, Recorder>,
        ) {
            let calls = caller(config, recorder);
            let mut fixture = Fixture::new($call_type);
            let answer = {
                let env = fixture.env.pre_post();
                let log = LogContext {
                    state: &mut fixture.state,
                    env: &env,
                };
                match calls.$method(request, log) {
                    Ok(work) => Ok($ready(work)),
                    Err(failure) => Err(failure),
                }
            };
            (answer, fixture, calls)
        }
    };
    (awaited $name:ident, $call_type:expr, $request:ident, $reply:ty, $method:ident) => {
        pub(crate) fn $name(
            config: RecordingConfig,
            recorder: Recorder,
            request: $request<'_>,
        ) -> (
            Result<$reply, EvalRunFailure>,
            Fixture,
            EvalRunCreateList<RecordingConfig, Recorder>,
        ) {
            let calls = caller(config, recorder);
            let mut fixture = Fixture::new($call_type);
            let answer = {
                let env = fixture.env.pre_post();
                let log = LogContext {
                    state: &mut fixture.state,
                    env: &env,
                };
                block_on(calls.$method(request, log))
            };
            (answer, fixture, calls)
        }
    };
}

call_helper!(sync sync_create, "create_run", CreateRunArgs, Run, create_run, ready);
call_helper!(sync sync_list, "list_runs", ListRunsArgs, ListRunsResponse, list_runs, ready_runs);
call_helper!(awaited awaited_create, "create_run", CreateRunArgs, Run, acreate_run);
call_helper!(awaited awaited_list, "list_runs", ListRunsArgs, ListRunsResponse, alist_runs);
