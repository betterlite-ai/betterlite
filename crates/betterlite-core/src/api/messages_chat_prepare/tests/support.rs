use std::collections::{BTreeMap, BTreeSet};
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
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry;
use url::Url;
use uuid::Uuid;

use crate::api::messages_chat_prepare::prepare::{DispatchFailure, PolyfillDispatch, PrepareError};
use crate::api::messages_compaction::dispatch::DispatchRequest;
use crate::api::messages_context_clear::counter::{TokenCountRequest, TokenCounter};
use crate::api::messages_context_clear::error::{ClearToolUsesError, ContextManagementError};
use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::summary_execution::completion::{
    ExecutorCompletion, SummaryCall, SummaryCallFailure, SummaryCompletion,
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

pub(crate) fn parse(raw: &str) -> Json {
    serde_json::from_str(raw).expect("the fixture parses as JSON")
}

pub(crate) fn decided(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).expect("the messages decode")
}

pub(crate) fn extra(raw: &str) -> betterlite_models::decisions::Extra {
    serde_json::from_str(raw).expect("the mapping decodes")
}

pub(crate) fn text_field<'value>(value: &'value Json, name: &str) -> Option<&'value str> {
    let Json::Object(fields) = value else {
        return None;
    };
    match fields.get(name)? {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// the client error a preparation must surface, failing loudly when the wrapper chose another variant
pub(crate) fn context_failure(error: PrepareError) -> ContextManagementError {
    match error {
        PrepareError::Context(failure) => failure,
        PrepareError::History(source) => panic!("unexpected history failure: {source}"),
        PrepareError::Shape(source) => panic!("unexpected shape failure: {source}"),
    }
}

pub(crate) fn principal(raw: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(raw).expect("principal fields");
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the principal builds")
}

// one dispatcher call as the wrapper made it, for both the scripted and the registered dispatcher
#[derive(Debug, Clone)]
pub(crate) struct DispatchRecord {
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) tools: Option<Vec<Json>>,
    pub(crate) system: Option<Json>,
    pub(crate) spec: Option<Json>,
    pub(crate) metadata: Option<Object>,
    pub(crate) has_router: bool,
    pub(crate) has_principal: bool,
}

impl DispatchRecord {
    fn of(request: &DispatchRequest<'_>) -> Self {
        Self {
            model: request.model.to_owned(),
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            system: request.system.clone(),
            spec: request.context_management_spec.cloned(),
            metadata: request.litellm_metadata.cloned(),
            has_router: request.llm_router.is_some(),
            has_principal: request.user_api_key_auth.is_some(),
        }
    }
}

fn calls(calls: &Mutex<Vec<DispatchRecord>>) -> Vec<DispatchRecord> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

// an injected dispatcher: it records its arguments and answers the one scripted outcome
pub(crate) struct ScriptedDispatch {
    answer: Result<Option<PolyfillResult>, DispatchFailure>,
    calls: Mutex<Vec<DispatchRecord>>,
}

impl ScriptedDispatch {
    pub(crate) fn new(answer: Result<Option<PolyfillResult>, DispatchFailure>) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<DispatchRecord> {
        calls(&self.calls)
    }
}

#[async_trait]
impl PolyfillDispatch for ScriptedDispatch {
    async fn dispatch(
        &self,
        request: DispatchRequest<'_>,
    ) -> Result<Option<PolyfillResult>, DispatchFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(DispatchRecord::of(&request));
        match &self.answer {
            Ok(result) => Ok(result.clone()),
            Err(error) => Err(error.clone()),
        }
    }
}

// the real dispatcher, wrapped so a test can read back the exact arguments B972 received
pub(crate) struct Recorder<Inner> {
    inner: Inner,
    calls: Mutex<Vec<DispatchRecord>>,
}

impl<Inner> Recorder<Inner> {
    pub(crate) fn new(inner: Inner) -> Self {
        Self {
            inner,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<DispatchRecord> {
        calls(&self.calls)
    }
}

#[async_trait]
impl<Inner> PolyfillDispatch for Recorder<Inner>
where
    Inner: PolyfillDispatch,
{
    async fn dispatch(
        &self,
        request: DispatchRequest<'_>,
    ) -> Result<Option<PolyfillResult>, DispatchFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(DispatchRecord::of(&request));
        self.inner.dispatch(request).await
    }
}

// the compact editor's count answers one number and hands back the conversation it saw
pub(crate) struct RecordingHistoryCounter {
    pub(crate) conversation: usize,
    seen: Mutex<Vec<Json>>,
}

impl RecordingHistoryCounter {
    pub(crate) fn new(conversation: usize) -> Self {
        Self {
            conversation,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn seen(&self) -> Vec<Json> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl HistoryTokenCounter for RecordingHistoryCounter {
    fn count(
        &self,
        _model: &str,
        input: CountedInput<'_>,
    ) -> impl Future<Output = Result<usize, SummaryHistoryError>> + Send {
        let count = match input {
            CountedInput::Conversation { messages, .. } => {
                let encoded: Json = serde_json::to_value(messages)
                    .and_then(serde_json::from_value)
                    .expect("the counted conversation encodes");
                self.seen
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(encoded);
                self.conversation
            }
            CountedInput::Text(_) => 0,
        };
        ready(Ok(count))
    }
}

// B968's sync editor only names a counter here; no scenario in these tests runs a clear edit for real
pub(crate) struct FixedClearCounter {
    pub(crate) count: usize,
}

impl TokenCounter for FixedClearCounter {
    fn count(&self, _request: TokenCountRequest<'_>) -> Result<usize, ClearToolUsesError> {
        Ok(self.count)
    }
}

// the policy seams never run while the proxy globals are absent, but the editor still names their types
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

// a router handle the wrapper only passes on; no scenario reads it through this type
pub(crate) struct NeverCalled;

#[async_trait]
impl SummaryCompletion for NeverCalled {
    async fn complete(
        &self,
        _call: SummaryCall<'_>,
    ) -> Result<CompletionAnswer, SummaryCallFailure> {
        Err(SummaryCallFailure {
            message: String::from("the wrapper never calls this router"),
        })
    }
}

// the proxy's router handle: the compact editor prefers it over the direct pipeline for its summary call
pub(crate) struct RouterCompletion<'inner> {
    pub(crate) inner: ExecutorCompletion<'inner>,
}

#[async_trait]
impl SummaryCompletion for RouterCompletion<'_> {
    async fn complete(
        &self,
        call: SummaryCall<'_>,
    ) -> Result<CompletionAnswer, SummaryCallFailure> {
        self.inner.complete(call).await
    }
}

// a scripted dispatcher answer whose applied_edits is empty, so a test can see it still wins over history
pub(crate) fn polyfilled(messages: Vec<Json>, system: Option<Json>) -> PolyfillResult {
    PolyfillResult {
        messages,
        system,
        applied_edits: Vec::new(),
        compaction_block: None,
        iterations_usage: None,
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
        random: Mutex::new(StdRng::seed_from_u64(11)),
        clock: || 1_700_000_000,
        next_id: || Uuid::from_u128(11),
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

// the tracing lines one run emitted, each prefixed with its level as the subscriber prints it
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
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{level} {}", message.0));
    }
}

pub(crate) fn capture_logs() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let subscriber = registry().with(logs.clone());
    let guard = set_default(subscriber);
    (logs, guard)
}
