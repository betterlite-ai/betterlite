use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::model_budget_limiter::enforcement::BudgetSubject;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::summary_model_access::{DEFAULT_SUMMARY_MAX_TOKENS, Settings};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::count::error::CountError;
use rust_decimal::Decimal;
use serde_json::Number;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry;

use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::summary_execution::completion::{
    SummaryCall, SummaryCallFailure, SummaryCompletion,
};
use crate::api::summary_execution::edit_spec::CompactEditSpec;
use crate::api::summary_execution::editor::{
    EditorRequest, EditorServices, apply_compact_20260112,
};
use crate::api::summary_execution::error::SummaryExecutionError;
use crate::api::summary_execution::extract::{
    CompletionAnswer, CompletionChoice, CompletionMessage, CompletionUsage,
};
use crate::api::summary_history::count::{CountedInput, HistoryTokenCounter};
use crate::api::summary_history::error::SummaryHistoryError;
use crate::api::summary_history::system::SystemPrompt;
use crate::api::summary_model_access::access::ProxyGlobals;
use crate::api::summary_model_access::checks::{ObjectCall, SummaryModelChecks};
use crate::api::summary_model_access::error::PolicyFailure;
use crate::api::summary_model_access::repositories::SummaryAccessRepositories;
use crate::api::summary_model_limits::budget::SummaryModelBudgets;
use crate::api::summary_model_limits::error::LimiterFailure;
use crate::api::summary_model_limits::fields::BudgetMap;
use crate::api::summary_model_limits::rate::{
    BaseDescriptorRequest, DescriptorScope, LimiterCapability, OVER_LIMIT,
    PARALLEL_REQUEST_LIMITER, ProxyHooks, ReadOnlyCheck, SummaryDescriptor, SummaryRateLimiter,
};

pub(crate) fn list(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).expect("a JSON list")
}

pub(crate) fn object(raw: &str) -> Object {
    serde_json::from_str(raw).expect("a JSON object")
}

pub(crate) fn spec(raw: &str) -> CompactEditSpec {
    serde_json::from_str(raw).expect("an edit spec")
}

pub(crate) fn principal(raw: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(raw).expect("principal fields");
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the principal builds")
}

// the order the editor's policy steps ran in, shared by the checks, the budgets and the hooks
#[derive(Clone, Default)]
pub(crate) struct Recorder(Arc<Mutex<Vec<&'static str>>>);

impl Recorder {
    fn record(&self, step: &'static str) {
        self.0.lock().unwrap().push(step);
    }

    pub(crate) fn steps(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
}

pub(crate) struct ScriptedChecks {
    pub(crate) recorder: Recorder,
    pub(crate) deny: bool,
}

impl ScriptedChecks {
    fn verdict(&self) -> Result<(), PolicyFailure<io::Error>> {
        self.recorder.record("access");
        if self.deny {
            Err(PolicyFailure::Denied)
        } else {
            Ok(())
        }
    }
}

impl SummaryModelChecks for ScriptedChecks {
    type Failure = io::Error;

    fn object_call(
        &self,
        _router: Option<&dyn ModelAccessView>,
        _call: &ObjectCall<'_>,
    ) -> Result<(), PolicyFailure<io::Error>> {
        self.verdict()
    }

    fn user_call(
        &self,
        _router: Option<&dyn ModelAccessView>,
        _model: &str,
        _user: &LiteLLMUserTable,
    ) -> Result<(), PolicyFailure<io::Error>> {
        self.verdict()
    }

    fn project_call(
        &self,
        _router: Option<&dyn ModelAccessView>,
        _model: &str,
        _project: &ProjectTable,
    ) -> Result<(), PolicyFailure<io::Error>> {
        self.verdict()
    }
}

pub(crate) struct NoRows;

impl SummaryAccessRepositories for NoRows {
    type Failure = io::Error;

    fn user(
        &self,
        _user_id: &str,
        _user_id_upsert: bool,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, io::Error>> + Send {
        ready(Ok(None))
    }

    fn project(
        &self,
        _project_id: &str,
    ) -> impl Future<Output = Result<Option<ProjectTable>, io::Error>> + Send {
        ready(Ok(None))
    }

    fn membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLLMTeamMembership>, io::Error>> + Send {
        ready(Ok(None))
    }
}

fn exceeded() -> ErrorClass {
    ErrorClass::BudgetExceeded(Box::new(BudgetExceededDetails {
        current_cost: Decimal::TWO,
        max_budget: Decimal::ONE,
        message: Some("exceeded".to_owned()),
        provider: String::new(),
        entity_type: None,
        entity_id: None,
        status_code: 429,
    }))
}

pub(crate) struct ScriptedBudgets {
    pub(crate) recorder: Recorder,
    pub(crate) deny: bool,
}

impl SummaryModelBudgets for ScriptedBudgets {
    fn within_model_budget(
        &self,
        _subject: &BudgetSubject<'_>,
        _model_max_budget: &BudgetMap,
        _model: &str,
    ) -> impl Future<Output = Result<(), ErrorClass>> + Send {
        self.recorder.record("budget");
        ready(if self.deny { Err(exceeded()) } else { Ok(()) })
    }
}

pub(crate) struct FakeLimiter {
    pub(crate) capable: bool,
    pub(crate) over: bool,
}

impl FakeLimiter {
    fn descriptor() -> SummaryDescriptor {
        SummaryDescriptor {
            key: String::from("key"),
            value: String::from("value"),
            rate_limit: None,
        }
    }
}

impl SummaryRateLimiter for FakeLimiter {
    type Failure = io::Error;

    fn supports(&self, _capability: LimiterCapability) -> bool {
        self.capable
    }

    fn create_rate_limit_descriptors(
        &self,
        _request: &BaseDescriptorRequest<'_>,
    ) -> Result<Vec<SummaryDescriptor>, LimiterFailure<io::Error>> {
        Ok(vec![Self::descriptor()])
    }

    fn add_scope_descriptors(
        &self,
        _scope: DescriptorScope,
        _principal: &UserAPIKeyAuth,
        _requested_model: &str,
        _descriptors: &mut Vec<SummaryDescriptor>,
    ) -> Result<(), LimiterFailure<io::Error>> {
        Ok(())
    }

    fn should_rate_limit(
        &self,
        _check: &ReadOnlyCheck<'_>,
    ) -> impl Future<Output = Result<Option<String>, LimiterFailure<io::Error>>> + Send {
        let code = if self.over {
            OVER_LIMIT.to_owned()
        } else {
            String::from("OK")
        };
        ready(Ok(Some(code)))
    }
}

pub(crate) struct ScriptedHooks {
    pub(crate) recorder: Recorder,
    pub(crate) limiter: FakeLimiter,
}

impl ProxyHooks for ScriptedHooks {
    type Limiter = FakeLimiter;
    type Error = io::Error;

    fn get_proxy_hook(&self, hook: &str) -> Result<Option<&FakeLimiter>, io::Error> {
        self.recorder.record("rate");
        if hook == PARALLEL_REQUEST_LIMITER {
            Ok(Some(&self.limiter))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Counted {
    Conversation {
        model: String,
        messages: Vec<Json>,
        tools: Option<Vec<Json>>,
    },
    Text(String),
}

pub(crate) struct ScriptedCounter {
    pub(crate) conversation: usize,
    pub(crate) text: usize,
    pub(crate) fail: bool,
    pub(crate) calls: Mutex<Vec<Counted>>,
}

impl Default for ScriptedCounter {
    fn default() -> Self {
        Self {
            conversation: 150_001,
            text: 0,
            fail: false,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl ScriptedCounter {
    pub(crate) fn calls(&self) -> Vec<Counted> {
        self.calls.lock().unwrap().clone()
    }

    pub(crate) fn texts(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Counted::Text(text) => Some(text),
                Counted::Conversation { .. } => None,
            })
            .collect()
    }
}

impl HistoryTokenCounter for ScriptedCounter {
    fn count(
        &self,
        model: &str,
        input: CountedInput<'_>,
    ) -> impl Future<Output = Result<usize, SummaryHistoryError>> + Send {
        let (recorded, tokens) = match input {
            CountedInput::Conversation { messages, tools } => (
                Counted::Conversation {
                    model: model.to_owned(),
                    messages: messages.to_vec(),
                    tools: tools.map(<[Json]>::to_vec),
                },
                self.conversation,
            ),
            CountedInput::Text(text) => (Counted::Text(text.to_owned()), self.text),
        };
        self.calls.lock().unwrap().push(recorded);
        let outcome = if self.fail {
            Err(SummaryHistoryError::TokenCount(CountError::MissingInput))
        } else {
            Ok(tokens)
        };
        ready(outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) max_tokens: u64,
    pub(crate) timeout_seconds: f64,
    pub(crate) metadata: Object,
    pub(crate) user: Option<String>,
    pub(crate) region: Option<String>,
}

pub(crate) enum Answer {
    Text(String),
    Raw(CompletionAnswer),
    Failure(String),
}

pub(crate) struct ScriptedCompletion {
    pub(crate) answer: Answer,
    pub(crate) calls: Mutex<Vec<RecordedCall>>,
}

impl ScriptedCompletion {
    pub(crate) fn text(answer: &str) -> Self {
        Self {
            answer: Answer::Text(answer.to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }

    pub(crate) fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait]
impl SummaryCompletion for ScriptedCompletion {
    async fn complete(
        &self,
        call: SummaryCall<'_>,
    ) -> Result<CompletionAnswer, SummaryCallFailure> {
        self.calls.lock().unwrap().push(RecordedCall {
            model: call.model.to_owned(),
            messages: call.messages.to_vec(),
            max_tokens: call.max_tokens,
            timeout_seconds: call.timeout_seconds,
            metadata: call.metadata.clone(),
            user: call.user.map(str::to_owned),
            region: call.allowed_model_region.map(str::to_owned),
        });
        match &self.answer {
            Answer::Text(text) => Ok(CompletionAnswer {
                choices: vec![CompletionChoice {
                    message: Some(CompletionMessage {
                        content: Some(Json::Text(text.clone())),
                    }),
                }],
                usage: Some(CompletionUsage {
                    prompt_tokens: Some(Json::Number(Number::from(12_u64))),
                    completion_tokens: Some(Json::Number(Number::from(3_u64))),
                }),
            }),
            Answer::Raw(answer) => Ok(answer.clone()),
            Answer::Failure(message) => Err(SummaryCallFailure {
                message: message.clone(),
            }),
        }
    }
}

pub(crate) struct Fixture {
    pub(crate) settings: Option<Settings>,
    pub(crate) attach_policy: bool,
    pub(crate) recorder: Recorder,
    pub(crate) checks: ScriptedChecks,
    pub(crate) repositories: NoRows,
    pub(crate) budgets: ScriptedBudgets,
    pub(crate) hooks: ScriptedHooks,
    pub(crate) counter: ScriptedCounter,
    pub(crate) router: ScriptedCompletion,
    pub(crate) direct: ScriptedCompletion,
}

impl Default for Fixture {
    fn default() -> Self {
        let recorder = Recorder::default();
        let answer = "<summary> NEW </summary>";
        Self {
            settings: Some(Settings {
                context_management_summary_model: Some(String::from("sum")),
                context_management_summary_max_tokens: DEFAULT_SUMMARY_MAX_TOKENS,
            }),
            attach_policy: true,
            checks: ScriptedChecks {
                recorder: recorder.clone(),
                deny: false,
            },
            repositories: NoRows,
            budgets: ScriptedBudgets {
                recorder: recorder.clone(),
                deny: false,
            },
            hooks: ScriptedHooks {
                recorder: recorder.clone(),
                limiter: FakeLimiter {
                    capable: true,
                    over: false,
                },
            },
            counter: ScriptedCounter::default(),
            router: ScriptedCompletion::text(answer),
            direct: ScriptedCompletion::text(answer),
            recorder,
        }
    }
}

impl Fixture {
    pub(crate) fn deny_access(&mut self) {
        self.checks.deny = true;
    }

    pub(crate) fn deny_budget(&mut self) {
        self.budgets.deny = true;
    }

    pub(crate) fn deny_rate(&mut self) {
        self.hooks.limiter.over = true;
    }

    pub(crate) fn summary_calls(&self) -> usize {
        self.router.count().saturating_add(self.direct.count())
    }
}

// which completion the editor reaches: the routed interface, or a direct pipeline the caller supplies
pub(crate) enum Port<'port> {
    Routed,
    Direct(&'port dyn SummaryCompletion),
}

pub(crate) async fn run(
    fixture: &Fixture,
    request: EditorRequest<'_>,
    port: Port<'_>,
) -> Result<PolyfillResult, SummaryExecutionError<io::Error>> {
    let proxy = ProxyGlobals {
        database: false,
        router: None,
        checks: &fixture.checks,
        repositories: &fixture.repositories,
    };
    let (router, direct): (Option<&dyn SummaryCompletion>, &dyn SummaryCompletion) = match port {
        Port::Routed => (Some(&fixture.router), &fixture.direct),
        Port::Direct(direct) => (None, direct),
    };
    let services = EditorServices {
        settings: fixture.settings.as_ref(),
        proxy: fixture.attach_policy.then_some(&proxy),
        budgets: fixture.attach_policy.then_some(&fixture.budgets),
        hooks: fixture.attach_policy.then_some(&fixture.hooks),
        counter: &fixture.counter,
        router,
        direct,
    };
    apply_compact_20260112(request, &services).await
}

// one caller request: its history, edit spec, system, metadata and principal
pub(crate) struct Sample {
    pub(crate) messages: Vec<Json>,
    pub(crate) spec: CompactEditSpec,
    pub(crate) system: Option<SystemPrompt>,
    pub(crate) tools: Option<Vec<Json>>,
    pub(crate) metadata: Option<Object>,
    pub(crate) principal: Option<UserAPIKeyAuth>,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            messages: list(
                r#"[
                    {"role":"user","content":[{"type":"compaction","content":"S"}]},
                    {"role":"assistant","content":"ack"},
                    {"role":"user","content":"recent question"}
                ]"#,
            ),
            spec: spec(
                r#"{"type":"compact_20260112","trigger":{"type":"input_tokens","value":50000}}"#,
            ),
            system: Some(SystemPrompt::Text(String::from("BASE SYSTEM"))),
            tools: None,
            metadata: None,
            principal: None,
        }
    }
}

impl Sample {
    pub(crate) fn request(&self, model: &str) -> EditorRequest<'_> {
        EditorRequest {
            model,
            messages: &self.messages,
            tools: self.tools.as_deref(),
            system: self.system.as_ref(),
            edit_spec: &self.spec,
            litellm_metadata: self.metadata.as_ref(),
            principal: self.principal.as_ref(),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    pub(crate) fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    pub(crate) fn matching(&self, needle: &str) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|line| line.contains(needle))
            .collect()
    }

    pub(crate) fn contains(&self, needle: &str) -> bool {
        !self.matching(needle).is_empty()
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        let level = *event.metadata().level();
        if matches!(level, Level::INFO | Level::WARN | Level::DEBUG) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{level} {}", message.0));
        }
    }
}

pub(crate) fn capture_logs() -> (Logs, tracing::subscriber::DefaultGuard) {
    let logs = Logs::default();
    let subscriber = registry().with(logs.clone());
    let guard = set_default(subscriber);
    (logs, guard)
}
