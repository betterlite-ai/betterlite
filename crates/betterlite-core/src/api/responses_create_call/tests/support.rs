use std::future::{self, Future};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use serde_json::value::RawValue;

use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_create::bridge::{Created, StampedResponse};
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create::error::{CreateError, CreateFailure};
use crate::api::responses_create::execute::mock_responses_api_response;
use crate::api::responses_create_http::service::{
    CreateLive, CreateSynthetic, LogContext, StreamSetup,
};
use crate::api::responses_json::{decision_json, json_value};
use crate::api::responses_parameters::prompt_merge::ReturnedMessage;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::prompting::{
    CARRY_UNMATCHED_MESSAGE_POINTS, HookRequest, MergeAnswer, MergeRequest, PromptManagerError,
};
use crate::api::responses_provider_selection::provider_config::{ConfigLookup, NativeConfig};
use crate::api::responses_provider_selection::resolution::ModelResolver;
use crate::api::responses_stream_driver::source::PrefetchedSource;

use super::super::aresponses;
use super::super::deps::{AsyncCreateDeps, AsyncPromptPort, CreateReturn, SyncHandoff};
use super::super::error::AsyncCreateError;

// The caller's opaque context survives the wrapper untouched, so the test can match it by name
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Token(pub(super) &'static str);

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the test JSON parses")
}

pub(super) fn value(text: &str) -> JsonValue {
    json_value(&json(text))
}

pub(super) fn call_from(text: &str) -> ResponsesCreateCall {
    transcode(&json(text)).expect("the test call converts")
}

pub(super) fn stamped(id: &str) -> Created<Token> {
    let mut response = mock_responses_api_response(Some("hi")).expect("the mock response builds");
    response.id = id.to_owned();
    Created::Responses(Box::new(StampedResponse {
        response,
        hidden: HiddenParams::new(),
    }))
}

pub(super) fn raw(text: &str) -> Created<Token> {
    Created::Raw(RawValue::from_string(String::from(text)).expect("the test raw value parses"))
}

fn stream_setup() -> StreamSetup {
    StreamSetup {
        model: String::from("openai/gpt-4o"),
        custom_llm_provider: String::from("openai"),
        litellm_metadata: None,
        request_data: None,
        call_type: None,
    }
}

pub(super) fn live() -> Created<Token> {
    Created::Live(Box::new(CreateLive {
        setup: stream_setup(),
        source: Box::new(PrefetchedSource::new(Vec::new())),
    }))
}

pub(super) fn synthetic() -> Created<Token> {
    Created::Synthetic(Box::new(CreateSynthetic {
        setup: stream_setup(),
        response: mock_responses_api_response(Some("hi")).expect("the mock response builds"),
    }))
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct Harness {
    pub(super) redactor: Redactor,
    pub(super) sink: Sink,
    pub(super) now: DateTime<FixedOffset>,
    pub(super) state: LoggingState,
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"gpt-4o","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds")
}

fn environment<'env>(
    redactor: &'env Redactor,
    log: &'env Sink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Resolution {
    pub(super) model: String,
    pub(super) custom_llm_provider: Option<String>,
    pub(super) api_base: Option<String>,
}

// The resolver records what aresponses asked it and answers the prefix the way the router would
#[derive(Default)]
pub(super) struct ScriptedResolver {
    requests: Mutex<Vec<Resolution>>,
}

impl ScriptedResolver {
    pub(super) fn requests(&self) -> Vec<Resolution> {
        let requests = self.requests.lock().expect("the resolver lock is free");
        requests.clone()
    }
}

impl ModelResolver for ScriptedResolver {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        let seen = Resolution {
            model: request.model.to_owned(),
            custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
            api_base: request.api_base.map(str::to_owned),
        };
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        requests.push(seen);
        drop(requests);
        let (provider, model) = if let Some(given) = request.custom_llm_provider {
            (given.to_owned(), strip(given, request.model).to_owned())
        } else {
            let Some((prefix, rest)) = request.model.split_once('/') else {
                return Err(ResolutionError::MissingProvider {
                    model: request.model.to_owned(),
                });
            };
            (prefix.to_owned(), rest.to_owned())
        };
        Ok(ResolvedModel {
            model,
            provider: ProviderIdentity::Custom(provider),
            api_base: None,
            api_key: None,
        })
    }
}

fn strip<'model>(provider: &str, model: &'model str) -> &'model str {
    match model.split_once('/') {
        Some((prefix, rest)) if prefix == provider => rest,
        Some(_) | None => model,
    }
}

// Only openai has a native Responses API here; every other provider takes the chat bridge
pub(super) struct Lookup;

impl ConfigLookup for Lookup {
    fn responses_api_config(
        &self,
        _model: &str,
        provider: &str,
        _api_base: Option<&str>,
    ) -> Option<NativeConfig> {
        (provider == "openai").then_some(NativeConfig {
            encrypted_agent_messages: true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HookRecord {
    pub(super) model: String,
    pub(super) prompt_id: Option<String>,
    pub(super) prompt_label: Option<String>,
    pub(super) prompt_version: Option<Json>,
    pub(super) prompt_variables: Option<Json>,
    pub(super) client_messages: Vec<Json>,
    pub(super) carry: bool,
    pub(super) aresponses: bool,
}

pub(super) enum PortAnswer {
    Answer(MergeAnswer),
    Fail(String),
}

// The async prompt hook LiteLLM's logging object runs, scripted and counted
pub(super) struct ScriptedPort {
    runs: bool,
    answer: PortAnswer,
    checks: AtomicUsize,
    checks_seen: Mutex<Vec<Option<String>>>,
    records: Mutex<Vec<HookRecord>>,
}

impl ScriptedPort {
    pub(super) fn answering(runs: bool, answer: PortAnswer) -> Self {
        Self {
            runs,
            answer,
            checks: AtomicUsize::new(0),
            checks_seen: Mutex::new(Vec::new()),
            records: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }

    pub(super) fn checked_prompt_ids(&self) -> Vec<Option<String>> {
        let seen = self.checks_seen.lock().expect("the check lock is free");
        seen.clone()
    }

    pub(super) fn records(&self) -> Vec<HookRecord> {
        let records = self.records.lock().expect("the record lock is free");
        records.clone()
    }
}

#[async_trait]
impl AsyncPromptPort for ScriptedPort {
    fn runs_prompt_hooks(&self, request: &HookRequest<'_>) -> bool {
        self.checks.fetch_add(1, Ordering::SeqCst);
        let mut seen = self.checks_seen.lock().expect("the check lock is free");
        seen.push(request.prompt_id.map(str::to_owned));
        drop(seen);
        self.runs
    }

    async fn async_merge_prompt(
        &self,
        request: &MergeRequest<'_>,
    ) -> Result<MergeAnswer, PromptManagerError> {
        let record = HookRecord {
            model: request.model.to_owned(),
            prompt_id: request.prompt_id.map(str::to_owned),
            prompt_label: request.prompt_label.map(str::to_owned),
            prompt_version: request.prompt_version.map(decision_json),
            prompt_variables: request.prompt_variables.map(decision_json),
            client_messages: request
                .messages
                .iter()
                .map(|message| decision_json(&message.message))
                .collect(),
            carry: request
                .request_kwargs
                .contains_key(CARRY_UNMATCHED_MESSAGE_POINTS),
            aresponses: request.non_default_params.contains_key("aresponses"),
        };
        let mut records = self.records.lock().expect("the record lock is free");
        records.push(record);
        drop(records);
        match &self.answer {
            PortAnswer::Answer(answer) => Ok(answer.clone()),
            PortAnswer::Fail(message) => Err(PromptManagerError {
                message: message.clone(),
            }),
        }
    }
}

pub(super) fn answer(model: &str, messages: &[&str]) -> MergeAnswer {
    MergeAnswer {
        model: model.to_owned(),
        messages: messages
            .iter()
            .map(|message| ReturnedMessage {
                source: None,
                message: transcode(&json(message)).expect("the message converts"),
            })
            .collect(),
        optional_params: ParameterMap::new(),
    }
}

pub(super) fn with_params(mut answer: MergeAnswer, params: &str) -> MergeAnswer {
    answer.optional_params = transcode(&json(params)).expect("the optional params convert");
    answer
}

pub(super) enum Outcome {
    Completed(Option<Created<Token>>),
    Pending(Option<Created<Token>>),
    Failed(CreateError),
}

struct ScriptedState {
    calls: Vec<ResponsesCreateCall>,
    contexts: Vec<String>,
    script: Option<Outcome>,
}

pub(super) struct ScriptedHandoff {
    state: Mutex<ScriptedState>,
    polls: Arc<AtomicUsize>,
}

impl ScriptedHandoff {
    pub(super) fn new(outcome: Outcome) -> Self {
        Self {
            state: Mutex::new(ScriptedState {
                calls: Vec::new(),
                contexts: Vec::new(),
                script: Some(outcome),
            }),
            polls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn calls(&self) -> Vec<ResponsesCreateCall> {
        let state = self.state.lock().expect("the handoff lock is free");
        state.calls.clone()
    }

    pub(super) fn contexts(&self) -> Vec<String> {
        let state = self.state.lock().expect("the handoff lock is free");
        state.contexts.clone()
    }

    pub(super) fn polls(&self) -> usize {
        self.polls.load(Ordering::SeqCst)
    }
}

struct ScriptedFuture {
    outcome: Option<Created<Token>>,
    polls: Arc<AtomicUsize>,
}

impl Future for ScriptedFuture {
    type Output = Result<Option<Created<Token>>, CreateError>;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let pinned = self.get_mut();
        let previous = pinned.polls.fetch_add(1, Ordering::SeqCst);
        assert!(previous <= 1, "the coroutine was awaited more than once");
        if previous == 0 {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(Ok(pinned.outcome.take()))
    }
}

impl SyncHandoff<Token> for ScriptedHandoff {
    fn handoff<'call>(
        &'call self,
        call: &'call ResponsesCreateCall,
        context: Token,
        _log: &'call mut LogContext<'_, '_>,
    ) -> CreateReturn<'call, Token>
    where
        Token: 'call,
    {
        let mut state = self.state.lock().expect("the handoff lock is free");
        state.calls.push(call.clone());
        state.contexts.push(context.0.to_owned());
        let script = state.script.take();
        drop(state);
        match script.expect("one handoff per run") {
            Outcome::Completed(outcome) => CreateReturn::Completed(outcome),
            Outcome::Failed(error) => CreateReturn::Pending(Box::pin(future::ready(Err(error)))),
            Outcome::Pending(outcome) => CreateReturn::Pending(Box::pin(ScriptedFuture {
                outcome,
                polls: Arc::clone(&self.polls),
            })),
        }
    }
}

pub(super) async fn drive(
    resolver: &ScriptedResolver,
    port: Option<&dyn AsyncPromptPort>,
    handoff: &ScriptedHandoff,
    call: &ResponsesCreateCall,
) -> Result<Created<Token>, AsyncCreateError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
    let mut harness = Harness {
        state: logging_state(&redactor),
        redactor,
        sink: Sink,
        now,
    };
    let environment = environment(&harness.redactor, &harness.sink, harness.now);
    let lookup = Lookup;
    let deps = AsyncCreateDeps {
        resolver,
        lookup: &lookup,
        prompts: port,
    };
    let mut log = LogContext {
        state: &mut harness.state,
        env: &environment,
    };
    aresponses(&deps, handoff, call, Token("ctx"), &mut log).await
}

pub(super) fn typed(created: &Created<Token>) -> &StampedResponse {
    match created {
        Created::Responses(stamped) => stamped,
        Created::Raw(_) | Created::Live(_) | Created::Synthetic(_) | Created::Chat(_) => {
            panic!("the run answers a typed response")
        }
    }
}

pub(super) fn input_json(call: &ResponsesCreateCall) -> Json {
    transcode(&call.input).expect("the handed-off input converts")
}

pub(super) fn failure(error: &AsyncCreateError) -> CreateFailure {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    error.sdk_failure(&redactor)
}
