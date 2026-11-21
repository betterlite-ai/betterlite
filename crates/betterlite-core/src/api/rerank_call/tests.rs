#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, pending, ready};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_config::api::rerank_provider_plan::RerankSettings;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use tokio::runtime::Builder;

use super::{AsyncRerankArgs, AsyncRerankPlan, PlanEntry, RerankEntry, RerankWork, arerank};
use crate::api::rerank_provider_plan::call::{RerankCall, ReturnDocuments};
use crate::api::rerank_provider_plan::dispatch::{
    OptionsRequest, Params, RerankOperations, SendRequest, provider_failure,
};
use crate::api::rerank_provider_plan::plan::{RerankException, RerankIdentity, RerankPlan};

const MODEL: &str = "cohere/rerank-english-v3.0";

#[derive(Debug, Clone)]
struct ResolutionSeen {
    model: String,
    custom_llm_provider: Option<String>,
    api_base: Option<String>,
}

struct FakeIdentity {
    resolved: Option<ResolvedModel>,
    requests: Mutex<Vec<ResolutionSeen>>,
}

impl FakeIdentity {
    fn new(model: &str, provider: ProviderIdentity, key: Option<&str>, base: Option<&str>) -> Self {
        Self {
            resolved: Some(ResolvedModel {
                model: model.to_owned(),
                provider,
                api_base: base.map(str::to_owned),
                api_key: key.map(|value| SecretString::from(value.to_owned())),
            }),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn resolved() -> Self {
        Self::new(
            "rerank-english-v3.0",
            ProviderIdentity::Known(ProviderId::Cohere),
            Some("dynamic-key"),
            Some("https://identity.test/v1"),
        )
    }

    fn missing() -> Self {
        Self {
            resolved: None,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ResolutionSeen> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RerankIdentity for FakeIdentity {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ResolutionSeen {
                model: request.model.to_owned(),
                custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
                api_base: request.api_base.map(str::to_owned),
            });
        self.resolved
            .clone()
            .ok_or_else(|| ResolutionError::MissingProvider {
                model: request.model.to_owned(),
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Immediate,
    Deferred,
    Pending,
    Failing,
    DeferredFailing,
}

#[derive(Debug, Clone)]
struct CallSeen {
    model: String,
    custom_llm_provider: Option<String>,
    top_n: Option<i64>,
    rank_fields: Option<Vec<String>>,
    return_documents: ReturnDocuments,
    max_chunks_per_doc: Option<i64>,
    max_tokens_per_doc: Option<i64>,
    kwargs: IndexMap<String, Json>,
    call_id: String,
}

struct FakeEntry {
    mode: Mode,
    begins: AtomicUsize,
    polls: AtomicUsize,
    cancelled: AtomicBool,
    calls: Mutex<Vec<CallSeen>>,
}

impl FakeEntry {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            begins: AtomicUsize::new(0),
            polls: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn begins(&self) -> usize {
        self.begins.load(Ordering::SeqCst)
    }

    fn polls(&self) -> usize {
        self.polls.load(Ordering::SeqCst)
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn calls(&self) -> Vec<CallSeen> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn worked(&self) -> Result<RerankWork<'_>, RerankException> {
        match self.mode {
            Mode::Immediate => Ok(RerankWork::Immediate(Box::new(reply()))),
            Mode::Deferred => Ok(RerankWork::Deferred(Box::pin(async move {
                self.polls.fetch_add(1, Ordering::SeqCst);
                Ok(reply())
            }))),
            Mode::Pending => Ok(RerankWork::Deferred(Box::pin(async move {
                self.polls.fetch_add(1, Ordering::SeqCst);
                let _guard = CancelOnDrop(&self.cancelled);
                pending::<Result<RerankResponse, RerankException>>().await
            }))),
            Mode::DeferredFailing => Ok(RerankWork::Deferred(Box::pin(async move {
                self.polls.fetch_add(1, Ordering::SeqCst);
                Err(rate_limited())
            }))),
            Mode::Failing => Err(rate_limited()),
        }
    }
}

impl RerankEntry for FakeEntry {
    fn begin<'call>(
        &'call self,
        call: RerankCall,
        logging: &'call mut LoggingState,
    ) -> Result<RerankWork<'call>, RerankException> {
        self.begins.fetch_add(1, Ordering::SeqCst);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CallSeen {
                model: call.model,
                custom_llm_provider: call.custom_llm_provider,
                top_n: call.top_n,
                rank_fields: call.rank_fields,
                return_documents: call.return_documents,
                max_chunks_per_doc: call.max_chunks_per_doc,
                max_tokens_per_doc: call.max_tokens_per_doc,
                kwargs: call.kwargs,
                call_id: logging.litellm_call_id.clone(),
            });
        self.worked()
    }
}

// Records that the facade dropped the coroutine it was waiting on, so a cancellation leaves no second call
struct CancelOnDrop<'flag>(&'flag AtomicBool);

impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone)]
struct Sent {
    model: String,
    provider: String,
    is_async: bool,
    max_tokens_per_doc: Option<i64>,
    return_documents: ReturnDocuments,
}

#[derive(Default)]
struct FakeOperations {
    sent: Mutex<Vec<Sent>>,
}

impl FakeOperations {
    fn sent(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RerankOperations for FakeOperations {
    fn map_options(&self, _request: OptionsRequest<'_>) -> Result<Params, Box<RawException>> {
        Ok(Params::new())
    }

    fn send(
        &self,
        request: SendRequest<'_>,
        _logging: &mut LoggingState,
    ) -> impl Future<Output = Result<RerankResponse, Box<RawException>>> + Send {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                model: request.model.to_owned(),
                provider: request.provider.to_owned(),
                is_async: request.is_async,
                max_tokens_per_doc: request.max_tokens_per_doc,
                return_documents: request.return_documents,
            });
        ready(Ok(reply()))
    }
}

fn call_args() -> AsyncRerankArgs {
    AsyncRerankArgs {
        model: String::from(MODEL),
        query: String::from("q"),
        documents: vec![
            RerankDocument::Text(String::from("a")),
            RerankDocument::Text(String::from("b")),
        ],
        custom_llm_provider: None,
        top_n: None,
        rank_fields: None,
        return_documents: None,
        max_chunks_per_doc: None,
        kwargs: IndexMap::new(),
    }
}

fn with(kwargs: &[(&str, Json)]) -> IndexMap<String, Json> {
    kwargs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn dispatcher_args() -> AsyncRerankArgs {
    let mut args = call_args();
    args.model = String::from("rerank-english-v3.0");
    args.custom_llm_provider = Some(String::from("cohere"));
    args.kwargs = with(&[
        ("api_key", Json::Text(String::from("sk-test"))),
        (
            "api_base",
            Json::Text(String::from("https://caller.test/v1")),
        ),
        ("max_tokens_per_doc", Json::Number(Number::from(8_i64))),
    ]);
    args
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

fn rate_limited() -> RerankException {
    let failure = provider_failure(
        429,
        "slow down",
        "rerank-english-v3.0",
        "cohere",
        "RateLimitError",
    );
    RerankException {
        class: dispatch(&DispatchInput::Raw(Box::new(failure)), &redactor()),
        message: String::from("slow down"),
    }
}

fn reply() -> RerankResponse {
    serde_json::from_str(
        r#"{"id":"r","results":[{"index":1,"relevance_score":0.9},{"index":0,"relevance_score":0.1}]}"#,
    )
    .expect("the fixture is a rerank response")
}

fn logging_state(call_id: &str) -> LoggingState {
    let moment =
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339");
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: String::from("rerank-v1"),
            call_type: String::from("rerank"),
            start_time: moment,
            litellm_call_id: call_id.to_owned(),
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
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

fn block_on<Output, Pending>(future: Pending) -> Output
where
    Pending: Future<Output = Output>,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds")
        .block_on(future)
}

fn run(
    identity: &FakeIdentity,
    entry: &FakeEntry,
    redactor: &Redactor,
    args: AsyncRerankArgs,
    logging: &mut LoggingState,
) -> Result<RerankResponse, RerankException> {
    let plan = AsyncRerankPlan {
        identity,
        entry,
        redactor,
    };
    block_on(arerank(&plan, args, logging))
}

#[test]
fn async_rerank_preserves_defaults() {
    let identity = FakeIdentity::resolved();
    let entry = FakeEntry::new(Mode::Immediate);
    let redactor = redactor();
    let mut logging = logging_state("rerank-context");
    let mut args = call_args();
    args.kwargs = with(&[
        ("max_tokens_per_doc", Json::Number(Number::from(8_i64))),
        ("arerank", Json::Bool(false)),
        (
            "api_base",
            Json::Text(String::from("https://caller.test/v1")),
        ),
    ]);
    let response =
        run(&identity, &entry, &redactor, args, &mut logging).expect("the default call answers");
    assert_eq!(response, reply());
    assert_eq!(entry.polls(), 0);
    let calls = entry.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, MODEL);
    assert_eq!(calls[0].custom_llm_provider, None);
    assert_eq!(calls[0].return_documents, ReturnDocuments::Null);
    assert_eq!(calls[0].max_tokens_per_doc, Some(8));
    assert_eq!(calls[0].kwargs.get("arerank"), Some(&Json::Bool(true)));
    assert!(!calls[0].kwargs.contains_key("max_tokens_per_doc"));
    let requests = identity.requests();
    assert_eq!(requests[0].model, MODEL);
    assert_eq!(requests[0].custom_llm_provider, None);
    assert_eq!(
        requests[0].api_base.as_deref(),
        Some("https://caller.test/v1")
    );
}

#[test]
fn async_rerank_passes_explicit_values() {
    let identity = FakeIdentity::resolved();
    let entry = FakeEntry::new(Mode::Immediate);
    let redactor = redactor();
    let mut logging = logging_state("rerank-context");
    let mut args = call_args();
    args.top_n = Some(0);
    args.rank_fields = Some(Vec::new());
    args.return_documents = Some(false);
    args.max_chunks_per_doc = Some(0);
    run(&identity, &entry, &redactor, args, &mut logging).expect("the explicit call answers");
    let calls = entry.calls();
    assert_eq!(calls[0].top_n, Some(0));
    assert_eq!(calls[0].rank_fields, Some(Vec::new()));
    assert_eq!(calls[0].return_documents, ReturnDocuments::Value(false));
    assert_eq!(calls[0].max_chunks_per_doc, Some(0));
    assert_eq!(calls[0].max_tokens_per_doc, None);
}

#[test]
fn async_rerank_awaits_one_result() {
    let identity = FakeIdentity::resolved();
    let entry = FakeEntry::new(Mode::Deferred);
    let redactor = redactor();
    let mut logging = logging_state("rerank-context");
    let response = run(&identity, &entry, &redactor, call_args(), &mut logging)
        .expect("the coroutine answers");
    assert_eq!(response, reply());
    assert_eq!(entry.begins(), 1);
    assert_eq!(entry.polls(), 1);
    assert_eq!(entry.calls()[0].call_id, "rerank-context");
}

#[test]
fn dropping_the_facade_cancels_the_pending_call() {
    let identity = FakeIdentity::resolved();
    let entry = FakeEntry::new(Mode::Pending);
    let redactor = redactor();
    let mut logging = logging_state("rerank-context");
    let plan = AsyncRerankPlan {
        identity: &identity,
        entry: &entry,
        redactor: &redactor,
    };
    let mut future = Box::pin(arerank(&plan, call_args(), &mut logging));
    let mut context = Context::from_waker(Waker::noop());
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    assert!(!entry.cancelled());
    drop(future);
    assert!(entry.cancelled());
    assert_eq!(entry.begins(), 1);
    assert_eq!(entry.polls(), 1);
}

#[test]
fn a_declared_authenticating_provider_skips_the_lookup() {
    let identity = FakeIdentity::missing();
    let entry = FakeEntry::new(Mode::Immediate);
    let redactor = redactor();
    let mut logging = logging_state("rerank-context");
    let mut declared = call_args();
    declared.model = String::from("github_copilot/rerank");
    run(&identity, &entry, &redactor, declared, &mut logging)
        .expect("the declared provider needs no lookup");
    assert!(identity.requests().is_empty());
    assert_eq!(entry.calls()[0].model, "github_copilot/rerank");
}

#[test]
fn async_rerank_attributes_errors() {
    let identity = FakeIdentity::missing();
    let entry = FakeEntry::new(Mode::Immediate);
    let redactor = redactor();
    let mut logging = logging_state("rerank-context");
    let mut args = call_args();
    args.model = String::from("rerank-english-v3.0");
    args.custom_llm_provider = Some(String::from("cohere"));
    args.kwargs = with(&[(
        "api_base",
        Json::Text(String::from("https://caller.test/v1")),
    )]);
    let error = run(&identity, &entry, &redactor, args, &mut logging)
        .expect_err("an unknown provider raises");
    let ErrorClass::BadRequest(details) = error.class else {
        panic!("the resolution failure is a bad request");
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.provider.as_deref(), Some(""));
    assert_eq!(details.model.as_deref(), Some("rerank-english-v3.0"));
    assert!(error.message.starts_with("LLM Provider NOT provided"));
    let requests = identity.requests();
    assert_eq!(requests[0].model, "rerank-english-v3.0");
    assert_eq!(requests[0].custom_llm_provider.as_deref(), Some("cohere"));
    assert_eq!(
        requests[0].api_base.as_deref(),
        Some("https://caller.test/v1")
    );
    assert_eq!(entry.begins(), 0);
}

#[test]
fn an_unresolved_provider_keeps_the_empty_label_and_names_no_openai() {
    let identity = FakeIdentity::missing();
    let entry = FakeEntry::new(Mode::Immediate);
    let redactor = redactor();
    let mut logging = logging_state("rerank-context");
    let mut args = call_args();
    args.model = String::from("rerank-english-v3.0");
    let error = run(&identity, &entry, &redactor, args, &mut logging)
        .expect_err("a model with no provider raises");
    let ErrorClass::BadRequest(details) = error.class else {
        panic!("the resolution failure is a bad request");
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.provider.as_deref(), Some(""));
    assert_eq!(entry.begins(), 0);
}

#[test]
fn a_non_text_api_base_is_refused_before_the_call() {
    let identity = FakeIdentity::resolved();
    let entry = FakeEntry::new(Mode::Immediate);
    let redactor = redactor();
    let mut logging = logging_state("rerank-context");
    let mut args = call_args();
    args.kwargs = with(&[("api_base", Json::Bool(true))]);
    let error = run(&identity, &entry, &redactor, args, &mut logging)
        .expect_err("a boolean api_base raises");
    assert_eq!(error.message, "api_base must be a string");
    let ErrorClass::BadRequest(details) = error.class else {
        panic!("the wrong-typed api_base is a bad request");
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.provider.as_deref(), Some(""));
    assert!(identity.requests().is_empty());
    assert_eq!(entry.begins(), 0);
}

#[test]
fn a_non_integer_max_tokens_per_doc_is_refused() {
    let identity = FakeIdentity::resolved();
    let entry = FakeEntry::new(Mode::Immediate);
    let redactor = redactor();
    let mut logging = logging_state("rerank-context");
    let mut args = call_args();
    args.kwargs = with(&[("max_tokens_per_doc", Json::Text(String::from("8")))]);
    let error = run(&identity, &entry, &redactor, args, &mut logging)
        .expect_err("a string max_tokens_per_doc raises");
    assert_eq!(error.message, "max_tokens_per_doc must be an integer");
    let ErrorClass::ApiConnection(context) = error.class else {
        panic!("the wrong-typed parameter is an api connection error");
    };
    assert_eq!(context.provider.as_deref(), Some("cohere"));
    assert_eq!(entry.begins(), 0);
}

#[rstest]
#[case::immediate_failure(Mode::Failing)]
#[case::deferred_failure(Mode::DeferredFailing)]
fn immediate_and_deferred_failures_keep_their_class(#[case] mode: Mode) {
    let identity = FakeIdentity::resolved();
    let redactor = redactor();
    let mut logging = logging_state("rerank-context");
    let entry = FakeEntry::new(mode);
    let error = run(&identity, &entry, &redactor, call_args(), &mut logging)
        .expect_err("the provider failure raises");
    assert!(matches!(error.class, ErrorClass::RateLimit(_)));
    assert_eq!(error.message, "slow down");
}

#[test]
fn plan_entry_runs_the_dispatcher() {
    let identity = FakeIdentity::resolved();
    let operations = FakeOperations::default();
    let settings = RerankSettings::default();
    let environment = BTreeMap::new();
    let redactor = redactor();
    let plan = RerankPlan {
        identity: &identity,
        operations: &operations,
        settings: &settings,
        environment: &environment,
        redactor: &redactor,
    };
    let entry = PlanEntry { plan: &plan };
    let facade = AsyncRerankPlan {
        identity: &identity,
        entry: &entry,
        redactor: &redactor,
    };
    let mut logging = logging_state("rerank-context");
    let response = block_on(arerank(&facade, dispatcher_args(), &mut logging))
        .expect("the dispatcher answers through the facade");
    assert_eq!(response, reply());
    let sent = operations.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].model, "rerank-english-v3.0");
    assert_eq!(sent[0].provider, "cohere");
    assert!(sent[0].is_async);
    assert_eq!(sent[0].max_tokens_per_doc, Some(8));
    assert_eq!(sent[0].return_documents, ReturnDocuments::Null);
}
