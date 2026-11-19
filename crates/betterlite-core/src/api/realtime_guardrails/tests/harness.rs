#![cfg(test)]

use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use crate::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use crate::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use crate::api::realtime_contracts::transport::{
    ClientWebSocket, RealtimeBackend, RealtimeMessage, ScopedWebSocket, WebSocketScope,
};
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_session_state::{ConnectionSetup, RealtimeStreaming};
use crate::api::request_destination::RequestData;

use super::super::callback::{
    GuardrailFailure, GuardrailInput, GuardrailInputType, GuardrailTexts, GuardrailViolation,
    RealtimeGuardrail, RealtimeGuardrailPolicy,
};
use super::super::error::RealtimeGuardrailsError;
use super::super::runner::{RealtimeGuardrails, RunRealtimeGuardrails as _};

pub(super) const PROMPT_PREFIX: &str =
    "Say exactly the following message to the user, word for word, do not add anything else: ";

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture frame is valid JSON")
}

pub(super) fn field<'value>(value: &'value Json, key: &str) -> &'value Json {
    match value {
        Json::Object(fields) => fields.get(key).expect("the fixture frame has the field"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture frame is not an object");
        }
    }
}

fn prompt_of(spoken: &Json) -> String {
    let item = field(spoken, "item");
    let Json::List(content) = field(item, "content") else {
        panic!("the spoken message content is a list");
    };
    match field(&content[0], "text") {
        Json::Text(prompt) => prompt.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the spoken message prompt is text");
        }
    }
}

fn principal() -> Json {
    Json::Object(IndexMap::from([(
        "user_id".to_owned(),
        Json::Text(String::from("u-1")),
    )]))
}

pub(super) fn check_data() -> RequestData {
    IndexMap::from([(
        String::from("transcript"),
        Json::Text(String::from("hello")),
    )])
}

pub(super) fn guardrail_input() -> GuardrailInput {
    GuardrailInput {
        texts: vec![String::from("hello")],
        images: Vec::new(),
        request_data: IndexMap::from([(String::from("user_api_key_dict"), principal())]),
        input_type: GuardrailInputType::Request,
    }
}

fn denied() -> GuardrailViolation {
    GuardrailViolation {
        detail: Json::Null,
        message: String::from("denied"),
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum TestFailure {
    #[error("the scripted guardrail predicate failed")]
    Predicate,
    #[error("the scripted guardrail raised an unexpected error")]
    Unexpected,
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted provider is not configured")]
pub(super) struct ConfigFailure;

#[derive(Clone, Default)]
pub(super) struct Split {
    pub(super) seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl BaseRealtimeConfig for Split {
    type Error = ConfigFailure;
    type Backend = MockBackend;

    fn validate_environment(
        &self,
        _environment: &Environment<'_>,
    ) -> Result<HeaderMap, ConfigFailure> {
        Ok(HeaderMap::new())
    }

    fn transform_realtime_request(
        &self,
        message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, ConfigFailure> {
        lock(&self.seen).push(message.to_owned());
        Ok(Vec::new())
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, ConfigFailure> {
        Err(ConfigFailure)
    }
}

#[derive(Clone, Default)]
pub(super) struct Timeline(Arc<Mutex<Vec<&'static str>>>);

impl Timeline {
    fn record(&self, entry: &'static str) {
        lock(&self.0).push(entry);
    }

    pub(super) fn entries(&self) -> Vec<&'static str> {
        lock(&self.0).clone()
    }
}

#[derive(Default)]
struct SendData {
    sent: Vec<String>,
    attempts: usize,
    failures: Vec<usize>,
}

#[derive(Clone, Default)]
pub(super) struct SendLog(Arc<Mutex<SendData>>);

impl SendLog {
    pub(super) fn failing(failures: Vec<usize>) -> Self {
        let log = Self::default();
        lock(&log.0).failures = failures;
        log
    }

    fn begin(&self, target: &str) -> Result<(), WebSocketError> {
        let mut data = lock(&self.0);
        let attempt = data.attempts;
        data.attempts = data.attempts.saturating_add(1);
        if data.failures.contains(&attempt) {
            return Err(WebSocketError {
                message: format!("the scripted {target} send {attempt} failed"),
            });
        }
        drop(data);
        Ok(())
    }

    fn push(&self, frame: String) {
        lock(&self.0).sent.push(frame);
    }

    pub(super) fn sent(&self) -> Vec<String> {
        lock(&self.0).sent.clone()
    }
}

pub(super) struct MockBackend {
    log: SendLog,
    timeline: Timeline,
    closes: Arc<AtomicUsize>,
    close_failure: bool,
}

#[async_trait]
impl RealtimeBackend for MockBackend {
    async fn send(&mut self, message: RealtimeMessage) -> Result<(), WebSocketError> {
        self.log.begin("backend")?;
        let RealtimeMessage::Text(text) = message else {
            return Err(WebSocketError {
                message: String::from("the guardrail runner never sends binary frames"),
            });
        };
        self.timeline.record("backend");
        self.log.push(text);
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted backend does not read"),
        })
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        self.timeline.record("backend-close");
        self.closes.fetch_add(1, Ordering::SeqCst);
        if self.close_failure {
            return Err(WebSocketError {
                message: String::from("the scripted backend close failed"),
            });
        }
        Ok(())
    }
}

pub(super) struct MockClient {
    log: SendLog,
    timeline: Timeline,
    closes: Arc<AtomicUsize>,
    scope: WebSocketScope,
}

#[async_trait]
impl ClientWebSocket for MockClient {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        self.log.begin("client")?;
        self.timeline.record("client");
        self.log.push(data.to_owned());
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the scripted client does not read"),
        })
    }

    async fn close(&mut self, _code: u16, _reason: Option<&str>) -> Result<(), WebSocketError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl ScopedWebSocket for MockClient {
    fn scope(&self) -> &WebSocketScope {
        &self.scope
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Sockets {
    Both,
    ClientOnly,
    BackendOnly,
}

pub(super) struct Fixture {
    pub(super) backend: SendLog,
    pub(super) client: SendLog,
    pub(super) timeline: Timeline,
    pub(super) backend_closes: Arc<AtomicUsize>,
    pub(super) client_closes: Arc<AtomicUsize>,
    pub(super) sockets: Sockets,
    pub(super) close_failure: bool,
    pub(super) request_data: RequestData,
    pub(super) user_api_key_dict: Option<Json>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            backend: SendLog::default(),
            client: SendLog::default(),
            timeline: Timeline::default(),
            backend_closes: Arc::new(AtomicUsize::new(0)),
            client_closes: Arc::new(AtomicUsize::new(0)),
            sockets: Sockets::Both,
            close_failure: false,
            request_data: IndexMap::new(),
            user_api_key_dict: None,
        }
    }

    pub(super) fn state(&self) -> RealtimeStreaming {
        let client: Option<Box<dyn ClientWebSocket>> = match self.sockets {
            Sockets::Both | Sockets::ClientOnly => Some(Box::new(MockClient {
                log: self.client.clone(),
                timeline: self.timeline.clone(),
                closes: Arc::clone(&self.client_closes),
                scope: WebSocketScope::default(),
            })),
            Sockets::BackendOnly => None,
        };
        let backend: Option<Box<dyn RealtimeBackend>> = match self.sockets {
            Sockets::Both | Sockets::BackendOnly => Some(Box::new(MockBackend {
                log: self.backend.clone(),
                timeline: self.timeline.clone(),
                closes: Arc::clone(&self.backend_closes),
                close_failure: self.close_failure,
            })),
            Sockets::ClientOnly => None,
        };
        RealtimeStreaming::new(ConnectionSetup {
            client,
            backend,
            logging: None,
            logging_worker: None,
            provider_config: None,
            event_normalizer: None,
            logged_real_time_event_types: LoggedEventTypes::Default,
            model: String::from("gpt-realtime"),
            user_api_key_dict: self.user_api_key_dict.clone(),
            request_data: Some(self.request_data.clone()),
            force_transcription_model: None,
            client_wants_beta: false,
            backend_uses_beta_protocol: None,
        })
    }
}

pub(super) fn frames(log: &SendLog) -> Vec<Json> {
    log.sent().iter().map(|text| json(text)).collect()
}

pub(super) fn closes(fixture: &Fixture) -> (usize, usize) {
    (
        fixture.backend_closes.load(Ordering::SeqCst),
        fixture.client_closes.load(Ordering::SeqCst),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Custom,
    Native,
    Logger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Predicate {
    Selected,
    Never,
    Fails,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Clean,
    Rewrite,
    Block,
    Unexpected,
}

#[derive(Default)]
struct Records {
    checks: Vec<(GuardrailEventHooks, RequestData)>,
    applied: Vec<GuardrailInput>,
}

pub(super) struct ScriptedGuardrail {
    kind: Kind,
    pub(super) predicate: Predicate,
    events: Vec<GuardrailEventHooks>,
    pub(super) outcome: Outcome,
    pub(super) violation: GuardrailViolation,
    pub(super) policy: RealtimeGuardrailPolicy,
    records: Arc<Mutex<Records>>,
}

impl ScriptedGuardrail {
    fn new(kind: Kind, events: &[GuardrailEventHooks], outcome: Outcome) -> Self {
        Self {
            kind,
            predicate: Predicate::Selected,
            events: events.to_vec(),
            outcome,
            violation: denied(),
            policy: RealtimeGuardrailPolicy::default(),
            records: Arc::new(Mutex::new(Records::default())),
        }
    }

    pub(super) fn records(&self) -> (Vec<(GuardrailEventHooks, RequestData)>, Vec<GuardrailInput>) {
        let records = lock(&self.records);
        (records.checks.clone(), records.applied.clone())
    }
}

impl RealtimeGuardrailCallback for ScriptedGuardrail {
    type Error = TestFailure;

    fn is_custom_guardrail(&self) -> bool {
        self.kind != Kind::Logger
    }

    fn should_run_guardrail(
        &self,
        data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, TestFailure> {
        lock(&self.records).checks.push((event, data.clone()));
        match self.predicate {
            Predicate::Selected => Ok(self.events.contains(&event)),
            Predicate::Never => Ok(false),
            Predicate::Fails => Err(TestFailure::Predicate),
        }
    }
}

impl RealtimeGuardrail for ScriptedGuardrail {
    fn use_native_lifecycle_hooks(&self) -> bool {
        self.kind == Kind::Native
    }

    fn policy(&self) -> &RealtimeGuardrailPolicy {
        &self.policy
    }

    fn apply_guardrail(
        &self,
        input: GuardrailInput,
    ) -> BoxFuture<'_, Result<GuardrailTexts, GuardrailFailure<TestFailure>>> {
        lock(&self.records).applied.push(input);
        let outcome = self.outcome;
        let violation = self.violation.clone();
        Box::pin(async move {
            match outcome {
                Outcome::Clean => Ok(GuardrailTexts::default()),
                Outcome::Rewrite => Ok(GuardrailTexts(vec![String::from("the guardrail rewrite")])),
                Outcome::Block => Err(GuardrailFailure::Blocked(violation)),
                Outcome::Unexpected => Err(GuardrailFailure::Unexpected(TestFailure::Unexpected)),
            }
        })
    }
}

pub(super) fn blocking_guardrail() -> ScriptedGuardrail {
    ScriptedGuardrail::new(
        Kind::Custom,
        &[GuardrailEventHooks::RealtimeInputTranscription],
        Outcome::Block,
    )
}

#[derive(Clone, Default)]
pub(super) struct Capture(Arc<Mutex<Vec<(Level, String)>>>);

impl Capture {
    pub(super) fn messages(&self) -> Vec<String> {
        lock(&self.0)
            .iter()
            .map(|(_level, message)| message.clone())
            .collect()
    }
}

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if !visitor.0.is_empty() {
            lock(&self.0).push((*event.metadata().level(), visitor.0));
        }
    }
}

pub(super) fn capture() -> (Capture, DefaultGuard) {
    let logs = Capture::default();
    let guard = set_default(registry().with(logs.clone()));
    (logs, guard)
}

pub(super) struct Selection {
    pub(super) blocked: bool,
    pub(super) count: usize,
    pub(super) checks: Vec<(GuardrailEventHooks, RequestData)>,
    pub(super) applied: Vec<GuardrailInput>,
    pub(super) skipped: usize,
    pub(super) false_checks: usize,
    pub(super) frames: usize,
}

struct SelectionInputs {
    fixture: Fixture,
    eligible: ScriptedGuardrail,
    native: ScriptedGuardrail,
    logger: ScriptedGuardrail,
    never: ScriptedGuardrail,
}

fn selection_inputs() -> SelectionInputs {
    let mut fixture = Fixture::new();
    fixture.request_data = IndexMap::from([(
        String::from("transcript"),
        Json::Text(String::from("stale")),
    )]);
    fixture.user_api_key_dict = Some(principal());
    let eligible = ScriptedGuardrail::new(
        Kind::Custom,
        &[
            GuardrailEventHooks::RealtimeInputTranscription,
            GuardrailEventHooks::PreCall,
        ],
        Outcome::Rewrite,
    );
    let native = ScriptedGuardrail::new(
        Kind::Native,
        &[GuardrailEventHooks::RealtimeInputTranscription],
        Outcome::Block,
    );
    let logger = ScriptedGuardrail::new(
        Kind::Logger,
        &[GuardrailEventHooks::RealtimeInputTranscription],
        Outcome::Block,
    );
    let mut never = ScriptedGuardrail::new(
        Kind::Custom,
        &[GuardrailEventHooks::RealtimeInputTranscription],
        Outcome::Block,
    );
    never.predicate = Predicate::Never;
    SelectionInputs {
        fixture,
        eligible,
        native,
        logger,
        never,
    }
}

pub(super) fn selection(hooks: Option<&[GuardrailEventHooks]>) -> BoxFuture<'_, Selection> {
    let inputs = selection_inputs();
    async move {
        let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![
            &inputs.eligible,
            &inputs.eligible,
            &inputs.native,
            &inputs.logger,
            &inputs.never,
        ];
        let mut state = inputs.fixture.state();
        let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
            state: &mut state,
            provider: None,
            guardrails: &guardrails,
        };
        let blocked = runner
            .run_realtime_guardrails("hello", None, None, hooks)
            .await
            .expect("a clean run returns");
        let count = runner.state.setup.violation_count;
        selection_summary(&inputs, blocked, count)
    }
    .boxed()
}

fn selection_summary(inputs: &SelectionInputs, blocked: bool, count: usize) -> Selection {
    let skipped: usize = [&inputs.native, &inputs.logger]
        .iter()
        .map(|guardrail| {
            let (checks, applied) = guardrail.records();
            checks.len().saturating_add(applied.len())
        })
        .sum();
    let false_checks = inputs.never.records().0.len();
    let (checks, applied) = inputs.eligible.records();
    Selection {
        blocked,
        count,
        checks,
        applied,
        skipped,
        false_checks,
        frames: inputs
            .fixture
            .backend
            .sent()
            .len()
            .saturating_add(inputs.fixture.client.sent().len()),
    }
}

pub(super) struct BlockedMessage {
    pub(super) message: Json,
    pub(super) prompt: String,
}

pub(super) fn blocked_message(
    violation: GuardrailViolation,
    configured: Option<&str>,
) -> BoxFuture<'_, BlockedMessage> {
    async move {
        let fixture = Fixture::new();
        let mut guardrail = blocking_guardrail();
        guardrail.violation = violation;
        guardrail.policy.violation_message = configured.map(str::to_owned);
        let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![&guardrail];
        let mut state = fixture.state();
        let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
            state: &mut state,
            provider: None,
            guardrails: &guardrails,
        };
        assert!(
            runner
                .run_realtime_guardrails("hello", None, None, None)
                .await
                .expect("the block completes")
        );
        let notice = json(&fixture.client.sent()[0]);
        let spoken = json(&fixture.backend.sent()[1]);
        BlockedMessage {
            message: field(field(&notice, "error"), "message").clone(),
            prompt: prompt_of(&spoken),
        }
    }
    .boxed()
}

pub(super) fn one_block<'run>(
    fixture: &'run Fixture,
    guardrail: &'run ScriptedGuardrail,
    transcript: &'run str,
) -> BoxFuture<'run, Result<bool, RealtimeGuardrailsError<ConfigFailure, TestFailure>>> {
    async move {
        let guardrails: Vec<&dyn RealtimeGuardrail<Error = TestFailure>> = vec![guardrail];
        let mut state = fixture.state();
        let mut runner: RealtimeGuardrails<'_, Split, TestFailure> = RealtimeGuardrails {
            state: &mut state,
            provider: None,
            guardrails: &guardrails,
        };
        runner
            .run_realtime_guardrails(transcript, None, None, None)
            .await
    }
    .boxed()
}
