use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::model_budget_limiter::enforcement::BudgetSubject;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::drop_params::DeploymentDropPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::{ChatDeployment, Provider};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_prompts::arguments::json::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use secrecy::SecretString;
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry;
use url::Url;
use uuid::Uuid;

use crate::api::messages_compaction::dispatch::{
    ContextEditor, DispatchRequest, EditCall, EditorOutcome,
};
use crate::api::messages_context_clear::constants::{CLEAR_TOOL_USES_EDIT_TYPE, COMPACT_EDIT_TYPE};
use crate::api::messages_context_clear::counter::{TokenCountRequest, TokenCounter};
use crate::api::messages_context_clear::error::ClearToolUsesError;
use crate::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, CompactionKind, PolyfillResult, UsageIteration, UsageKind,
};
use crate::api::summary_execution::completion::{
    SummaryCall, SummaryCallFailure, SummaryCompletion,
};
use crate::api::summary_execution::extract::CompletionAnswer;
use crate::api::summary_history::count::{CountedInput, HistoryTokenCounter};
use crate::api::summary_history::error::SummaryHistoryError;
use crate::api::summary_model_access::checks::{ObjectCall, SummaryModelChecks};
use crate::api::summary_model_access::error::PolicyFailure;
use crate::api::summary_model_access::repositories::SummaryAccessRepositories;
use crate::api::summary_model_limits::budget::SummaryModelBudgets;
use crate::api::summary_model_limits::error::LimiterFailure;
use crate::api::summary_model_limits::fields::BudgetMap;
use crate::api::summary_model_limits::rate::{
    BaseDescriptorRequest, DescriptorScope, LimiterCapability, ProxyHooks, ReadOnlyCheck,
    SummaryDescriptor, SummaryRateLimiter,
};
use crate::mock::{chat, embedding};

// a scripted editor result, mirroring the two native return shapes and a refusal
pub(crate) enum ScriptedResult {
    Legacy {
        messages: Vec<Json>,
        applied: Option<AppliedEdit>,
    },
    Polyfilled(PolyfillResult),
    Fail {
        status_code: u16,
        message: String,
    },
}

#[derive(Debug, Error)]
pub(crate) enum ScriptedError {
    #[error("{message}")]
    Refused { status_code: u16, message: String },
    #[error("{message}")]
    Exhausted { message: String },
}

#[derive(serde::Serialize)]
struct ClearRecord {
    editor: &'static str,
    model: String,
    messages: Vec<Json>,
    tools: Option<Vec<Json>>,
    system: Option<Json>,
    edit_spec: Json,
}

#[derive(serde::Serialize)]
struct CompactRecord {
    editor: &'static str,
    model: String,
    messages: Vec<Json>,
    tools: Option<Vec<Json>>,
    system: Option<Json>,
    edit_spec: Json,
    litellm_metadata: Option<Object>,
    llm_router: Option<&'static str>,
    user_api_key_auth: Option<&'static str>,
}

// the injected registry: each call records its arguments and answers the next scripted result
pub(crate) struct ScriptedEditor {
    pub(crate) clear: Mutex<VecDeque<ScriptedResult>>,
    pub(crate) compact: Mutex<VecDeque<ScriptedResult>>,
    pub(crate) calls: Mutex<Vec<Json>>,
}

impl ScriptedEditor {
    pub(crate) fn new(clear: Vec<ScriptedResult>, compact: Vec<ScriptedResult>) -> Self {
        Self {
            clear: Mutex::new(VecDeque::from(clear)),
            compact: Mutex::new(VecDeque::from(compact)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Json> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record<Record>(&self, record: &Record)
    where
        Record: serde::Serialize,
    {
        let encoded: Json = serde_json::to_value(record)
            .and_then(serde_json::from_value)
            .expect("a recorded call encodes");
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(encoded);
    }

    fn next(
        &self,
        scripts: &Mutex<VecDeque<ScriptedResult>>,
        editor: &str,
    ) -> Result<EditorOutcome, ScriptedError> {
        let script = scripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match script {
            Some(ScriptedResult::Legacy { messages, applied }) => {
                Ok(EditorOutcome::Legacy { messages, applied })
            }
            Some(ScriptedResult::Polyfilled(result)) => Ok(EditorOutcome::Polyfilled(result)),
            Some(ScriptedResult::Fail {
                status_code,
                message,
            }) => Err(ScriptedError::Refused {
                status_code,
                message,
            }),
            None => Err(ScriptedError::Exhausted {
                message: format!("the case ran out of {editor} scripts"),
            }),
        }
    }
}

impl ContextEditor for ScriptedEditor {
    type Failure = ScriptedError;

    fn edit(
        &self,
        call: EditCall<'_>,
    ) -> impl Future<Output = Result<EditorOutcome, ScriptedError>> + Send {
        let outcome = match call {
            EditCall::Clear(clear) => {
                let record = ClearRecord {
                    editor: CLEAR_TOOL_USES_EDIT_TYPE,
                    model: clear.model,
                    messages: clear.messages,
                    tools: clear.tools,
                    system: clear.system,
                    edit_spec: clear.edit_spec,
                };
                self.record(&record);
                self.next(&self.clear, "clear_tool_uses")
            }
            EditCall::Compact(compact) => {
                let record = CompactRecord {
                    editor: COMPACT_EDIT_TYPE,
                    model: compact.model.to_owned(),
                    messages: compact.messages,
                    tools: compact.tools,
                    system: compact.system,
                    edit_spec: compact.edit_spec,
                    litellm_metadata: compact.litellm_metadata.cloned(),
                    llm_router: compact.llm_router.map(|_router| "router"),
                    user_api_key_auth: compact.user_api_key_auth.map(|_principal| "present"),
                };
                self.record(&record);
                self.next(&self.compact, "compact")
            }
        };
        ready(outcome)
    }
}

// a scripted counter for B968's sync editor
pub(crate) struct FixedClearCounter {
    pub(crate) count: usize,
}

impl TokenCounter for FixedClearCounter {
    fn count(&self, _request: TokenCountRequest<'_>) -> Result<usize, ClearToolUsesError> {
        Ok(self.count)
    }
}

// a scripted counter for B2432's effective-token count: conversations and extra texts answer separately
pub(crate) struct FixedHistoryCounter {
    pub(crate) conversation: usize,
    pub(crate) text: usize,
}

impl HistoryTokenCounter for FixedHistoryCounter {
    fn count(
        &self,
        _model: &str,
        input: CountedInput<'_>,
    ) -> impl Future<Output = Result<usize, SummaryHistoryError>> + Send {
        let count = match input {
            CountedInput::Conversation { .. } => self.conversation,
            CountedInput::Text(_) => self.text,
        };
        ready(Ok(count))
    }
}

// the policy seams never run while the principal is absent, but the editor still names their types
pub(crate) struct NoChecks;

type Access<'router> = Option<&'router dyn ModelAccessView>;
type UserRow<'row> = &'row LiteLLMUserTable;
type ProjectRow<'row> = &'row ProjectTable;
type Check = Result<(), PolicyFailure<io::Error>>;

impl SummaryModelChecks for NoChecks {
    type Failure = io::Error;

    fn object_call(&self, _router: Access<'_>, _call: &ObjectCall<'_>) -> Check {
        Ok(())
    }

    fn user_call(&self, _router: Access<'_>, _model: &str, _user: UserRow<'_>) -> Check {
        Ok(())
    }

    fn project_call(&self, _router: Access<'_>, _model: &str, _project: ProjectRow<'_>) -> Check {
        Ok(())
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

pub(crate) struct NoBudgets;

impl SummaryModelBudgets for NoBudgets {
    fn within_model_budget(
        &self,
        _subject: &BudgetSubject<'_>,
        _model_max_budget: &BudgetMap,
        _model: &str,
    ) -> impl Future<Output = Result<(), ErrorClass>> + Send {
        ready(Ok(()))
    }
}

pub(crate) struct NoLimiter;

impl SummaryRateLimiter for NoLimiter {
    type Failure = io::Error;

    fn supports(&self, _capability: LimiterCapability) -> bool {
        false
    }

    fn create_rate_limit_descriptors(
        &self,
        _request: &BaseDescriptorRequest<'_>,
    ) -> Result<Vec<SummaryDescriptor>, LimiterFailure<io::Error>> {
        Ok(Vec::new())
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
        ready(Ok(None))
    }
}

pub(crate) struct NoHooks;

impl ProxyHooks for NoHooks {
    type Limiter = NoLimiter;
    type Error = io::Error;

    fn get_proxy_hook(&self, _hook: &str) -> Result<Option<&NoLimiter>, io::Error> {
        Ok(None)
    }
}

// the real chat executor behind ExecutorCompletion, pointed at the case's loopback provider
pub(crate) fn executor() -> ChatExecutor {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ChatExecutor {
        client: ProxiedClient::outbound(&policy).expect("the outbound client builds"),
        deployments: Vec::new(),
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::from([String::from("gpt-4o-mini")]),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(7)),
        clock: || 1_700_000_000,
        next_id: || Uuid::from_u128(7),
        mock: MockSettings::default(),
        mock_chat: chat::response,
        mock_embedding: embedding::response,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
        bundled_catalog: Arc::default(),
    }
}

pub(crate) fn deployment(api_base: &str) -> ChatDeployment {
    ChatDeployment {
        model_group: String::from("main-model"),
        litellm_model: String::from("openai/gpt-4o-mini"),
        provider: Provider::OpenAi {
            model: String::from("gpt-4o-mini"),
        },
        api_base: api_base.to_owned(),
        endpoint: Url::parse(&format!("{api_base}/chat/completions"))
            .expect("the endpoint is a URL"),
        authorization: None,
        api_key: Some(SecretString::from("sk-test")),
        organization: None,
        timeout: Duration::from_secs(60),
        timeout_seconds: 60.0,
        model_id: String::from("summary"),
        weight: 0,
        rpm: 0,
        tpm: 0,
        global_drop_params: false,
        drop_policy: DeploymentDropPolicy::default(),
    }
}

// the few fields a test reads back out of a recorded call or a request body
pub(crate) fn object_keys(value: &Json) -> BTreeSet<String> {
    match value {
        Json::Object(fields) => fields.keys().cloned().collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            BTreeSet::new()
        }
    }
}

pub(crate) fn names(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

pub(crate) fn parse(raw: &str) -> Json {
    serde_json::from_str(raw).expect("the fixture parses as JSON")
}

pub(crate) fn field<'value>(value: &'value Json, name: &str) -> Option<&'value Json> {
    match value {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn text_field<'value>(value: &'value Json, name: &str) -> Option<&'value str> {
    match field(value, name)? {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(crate) fn decided(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).expect("the messages decode")
}

pub(crate) fn applied(cleared_tool_uses: usize) -> AppliedEdit {
    AppliedEdit {
        edit_type: Some(String::from(CLEAR_TOOL_USES_EDIT_TYPE)),
        cleared_input_tokens: None,
        cleared_tool_uses: Some(cleared_tool_uses),
        cleared_thinking_turns: None,
        summary_input_tokens: None,
        summary_output_tokens: None,
        error: None,
        warnings: None,
    }
}

pub(crate) fn block(content: &str) -> CompactionBlock {
    CompactionBlock {
        kind: CompactionKind::Compaction,
        content: Some(content.to_owned()),
        signature: None,
    }
}

pub(crate) fn iteration(input_tokens: u64, output_tokens: u64) -> UsageIteration {
    UsageIteration {
        kind: UsageKind::Compaction,
        input_tokens: Some(input_tokens),
        output_tokens: Some(output_tokens),
    }
}

pub(crate) fn polyfilled(
    messages: Vec<Json>,
    system: Option<Json>,
    applied_edits: Vec<AppliedEdit>,
    compaction_block: Option<CompactionBlock>,
    iterations_usage: Option<Vec<UsageIteration>>,
) -> PolyfillResult {
    PolyfillResult {
        messages,
        system,
        applied_edits,
        compaction_block,
        iterations_usage,
    }
}

pub(crate) fn principal(raw: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(raw).expect("principal fields");
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the principal builds")
}

// a router the dispatcher must only pass along: the scripted editor never calls it
pub(crate) struct NeverCalled;

#[async_trait]
impl SummaryCompletion for NeverCalled {
    async fn complete(
        &self,
        _call: SummaryCall<'_>,
    ) -> Result<CompletionAnswer, SummaryCallFailure> {
        Err(SummaryCallFailure {
            message: String::from("the dispatcher never calls the router"),
        })
    }
}

pub(crate) fn request<'request>(
    messages: Vec<Json>,
    system: Option<Json>,
    spec: Option<&'request Json>,
) -> DispatchRequest<'request> {
    DispatchRequest {
        model: "main-model",
        messages,
        tools: Some(vec![parse(r#"{"name":"lookup"}"#)]),
        system,
        context_management_spec: spec,
        litellm_metadata: None,
        llm_router: None,
        user_api_key_auth: None,
    }
}

// the tracing lines one run emitted, level-prefixed as the subscriber formats them
#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    pub(crate) fn lines(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn matching(&self, needle: &str) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|line| line.contains(needle))
            .collect()
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
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("{level} {}", message.0));
        }
    }
}

pub(crate) fn capture_logs() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let subscriber = registry().with(logs.clone());
    let guard = set_default(subscriber);
    (logs, guard)
}
