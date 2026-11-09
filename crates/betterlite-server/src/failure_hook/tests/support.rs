use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use tokio::task::yield_now;
use tokio_util::task::TaskTracker;

use super::super::alerts::ProxyAlerts;
use super::super::hook::FailureHook;
use super::super::lift::{ModelCallDetails, call_type_name};
use super::super::ports::{
    AlertLevel, AlertType, AlertingClient, BudgetAlertKind, CallEnvironment, CallInfo, Clock,
    DeploymentCatalog, EmailAlerting, EntityType, FailureInput, FailureLogging, FailureService,
    FailureTokenCounter, ListedDeployment, LoggingFactory, RouteCallTypes, SentryCapture,
    ServiceLogging, SlackAlert, SlackAlerting, TokenCountError, UsageCache,
};
use crate::guardrails::callback_caps::errors::{HttpException, ProxyHookError};
use crate::guardrails::callback_caps::traits::{
    FailureReply, GuardrailCallback, HookOverrides, ProxyCallback,
};

#[derive(Clone, Default)]
pub(super) struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    pub(super) fn record(&self, entry: String) {
        self.0.lock().expect("journal lock").push(entry);
    }

    pub(super) fn entries(&self) -> Vec<String> {
        self.0.lock().expect("journal lock").clone()
    }
}

#[derive(Default)]
struct LoggingState {
    details: ModelCallDetails,
    environment: Option<CallEnvironment>,
}

pub(super) struct FakeLogging {
    journal: Journal,
    baseline: bool,
    langfuse: Option<String>,
    on_langfuse: Option<Arc<dyn Fn() + Send + Sync>>,
    state: Mutex<LoggingState>,
}

impl FakeLogging {
    pub(super) fn new(journal: &Journal, baseline: bool, langfuse: Option<String>) -> Self {
        Self {
            journal: journal.clone(),
            baseline,
            langfuse,
            on_langfuse: None,
            state: Mutex::default(),
        }
    }

    pub(super) fn with_call_type(self, call_type: CallType) -> Self {
        self.state.lock().expect("state lock").details.call_type = call_type_name(call_type);
        self
    }

    pub(super) fn with_langfuse_hook(mut self, hook: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.on_langfuse = Some(hook);
        self
    }

    pub(super) fn set_details(&self, details: ModelCallDetails) {
        self.state.lock().expect("state lock").details = details;
    }

    pub(super) fn environment(&self) -> Option<CallEnvironment> {
        self.state.lock().expect("state lock").environment.clone()
    }

    pub(super) fn snapshot(&self) -> ModelCallDetails {
        self.state.lock().expect("state lock").details.clone()
    }
}

#[async_trait]
impl FailureLogging for FakeLogging {
    fn details(&self) -> ModelCallDetails {
        self.snapshot()
    }

    fn call_type(&self) -> Option<String> {
        self.state
            .lock()
            .expect("state lock")
            .details
            .call_type
            .clone()
    }

    fn set_call_type(&self, call_type: CallType) {
        self.journal.record(format!("call_type:{call_type:?}"));
        self.state.lock().expect("state lock").details.call_type = call_type_name(call_type);
    }

    fn record_input(&self, input: &FailureInput) {
        self.journal
            .record(format!("input:{}", input.detail_name()));
        if let FailureInput::Messages(messages) = input {
            self.state.lock().expect("state lock").details.messages = Some(messages.clone());
        }
    }

    fn mark_no_upstream_llm_call(&self) {
        self.journal.record("no_upstream_call".to_owned());
        self.state
            .lock()
            .expect("state lock")
            .details
            .no_upstream_llm_call = true;
    }

    fn update_environment_variables(&self, environment: CallEnvironment) {
        self.journal.record(format!(
            "environment:{}:{}:{:?}",
            environment.model, environment.user, environment.custom_llm_provider
        ));
        let mut state = self.state.lock().expect("state lock");
        state.details.model = Some(environment.model.clone());
        state.details.optional_params = Some(environment.optional_params.clone());
        state.environment = Some(environment);
    }

    fn has_baseline_context(&self) -> bool {
        self.baseline
    }

    async fn invalidate_baseline_estimate(&self, reason: &str, completed: bool) {
        self.journal
            .record(format!("invalidate:{reason}:{completed}"));
    }

    async fn pre_call(&self, input: &FailureInput, api_key: &str) {
        self.journal
            .record(format!("pre_call:{}:{api_key}", input.detail_name()));
    }

    async fn async_failure_handler(&self, _error: &ProxyHookError, traceback: &str) {
        self.journal
            .record(format!("async_failure:{}", !traceback.is_empty()));
    }

    fn failure_handler(&self, _error: &ProxyHookError, traceback: &str) {
        self.journal
            .record(format!("sync_failure:{}", !traceback.is_empty()));
    }

    async fn langfuse_trace_url(&self) -> Option<String> {
        yield_now().await;
        if let Some(hook) = &self.on_langfuse {
            hook();
        }
        self.langfuse.clone()
    }
}

pub(super) struct FakeFactory {
    journal: Journal,
    baseline: bool,
    langfuse: Option<String>,
    created: Mutex<Vec<Arc<FakeLogging>>>,
}

impl FakeFactory {
    pub(super) fn new(journal: &Journal) -> Self {
        let shared = journal.clone();
        Self {
            journal: shared,
            baseline: false,
            langfuse: None,
            created: Mutex::default(),
        }
    }

    pub(super) fn created(&self) -> Vec<Arc<FakeLogging>> {
        self.created.lock().expect("created lock").clone()
    }
}

impl LoggingFactory for FakeFactory {
    fn create_logging(
        &self,
        route: &str,
        call_id: &str,
        request_data: &JsonMap,
    ) -> Arc<dyn FailureLogging> {
        self.journal
            .record(format!("create:{route}:{call_id}:{}", request_data.len()));
        let logging = Arc::new(FakeLogging::new(
            &self.journal,
            self.baseline,
            self.langfuse.clone(),
        ));
        let mut logging_details = details();
        logging_details.call_type = Some(route.to_owned());
        logging.set_details(logging_details);
        self.created
            .lock()
            .expect("created lock")
            .push(Arc::clone(&logging));
        logging
    }
}

pub(super) struct FakeCounter {
    journal: Journal,
    text_tokens: i64,
    fail: bool,
}

impl FakeCounter {
    pub(super) fn new(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
            text_tokens: 3,
            fail: false,
        }
    }

    pub(super) fn failing(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
            text_tokens: 0,
            fail: true,
        }
    }

    pub(super) fn returning(journal: &Journal, text_tokens: i64) -> Self {
        Self {
            journal: journal.clone(),
            text_tokens,
            fail: false,
        }
    }
}

#[async_trait]
impl FailureTokenCounter for FakeCounter {
    async fn count_text(&self, model: &str, text: &str) -> Result<i64, TokenCountError> {
        self.journal.record(format!("count_text:{model}:{text}"));
        if self.fail {
            return Err(TokenCountError);
        }
        Ok(self.text_tokens)
    }

    async fn count_messages(
        &self,
        model: &str,
        messages: &[JsonValue],
    ) -> Result<i64, TokenCountError> {
        self.journal
            .record(format!("count_messages:{model}:{}", messages.len()));
        if self.fail {
            return Err(TokenCountError);
        }
        Ok(11)
    }
}

#[derive(Default)]
pub(super) struct FakeRoutes {
    table: BTreeMap<String, Vec<CallType>>,
}

impl FakeRoutes {
    pub(super) fn with(mut self, route: &str, call_types: Vec<CallType>) -> Self {
        self.table.insert(route.to_owned(), call_types);
        self
    }
}

impl RouteCallTypes for FakeRoutes {
    fn call_types_for(&self, route: &str) -> Option<Vec<CallType>> {
        self.table.get(route).cloned()
    }
}

pub(super) struct FakeCatalog {
    journal: Journal,
    groups: Vec<(String, Vec<ListedDeployment>)>,
}

impl FakeCatalog {
    pub(super) fn new(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
            groups: Vec::new(),
        }
    }

    pub(super) fn with(mut self, group: &str, deployments: Vec<ListedDeployment>) -> Self {
        self.groups.push((group.to_owned(), deployments));
        self
    }
}

impl DeploymentCatalog for FakeCatalog {
    fn get_model_list(&self, model_group: &str, team_id: Option<&str>) -> Vec<ListedDeployment> {
        self.journal
            .record(format!("model_list:{model_group}:{team_id:?}"));
        self.groups
            .iter()
            .find(|(group, _)| group == model_group)
            .map(|(_, deployments)| deployments.clone())
            .unwrap_or_default()
    }
}

#[derive(Default)]
pub(super) struct FakeUsageCache {
    journal: Journal,
}

impl FakeUsageCache {
    pub(super) fn new(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
        }
    }
}

#[async_trait]
impl UsageCache for FakeUsageCache {
    async fn set_local(&self, key: &str, value: &str, ttl: Duration) {
        self.journal
            .record(format!("status:{key}:{value}:{}", ttl.as_secs()));
    }
}

#[derive(Clone)]
pub(super) struct SlackRecord {
    pub message: String,
    pub level: AlertLevel,
    pub alert_type: AlertType,
    pub alerting_metadata: JsonMap,
    pub langfuse_trace: Option<String>,
}

#[derive(Default)]
struct SlackState {
    sent: Vec<SlackRecord>,
    budget: Vec<BudgetAlertKind>,
    failed_tracking: Vec<(String, String)>,
    too_long: Vec<Option<JsonMap>>,
}

pub(super) struct FakeSlack {
    journal: Journal,
    configured: bool,
    state: Mutex<SlackState>,
}

impl FakeSlack {
    pub(super) fn new(journal: &Journal, configured: bool) -> Self {
        Self {
            journal: journal.clone(),
            configured,
            state: Mutex::default(),
        }
    }

    pub(super) fn sent(&self) -> Vec<SlackRecord> {
        self.state.lock().expect("state lock").sent.clone()
    }

    pub(super) fn budget(&self) -> Vec<BudgetAlertKind> {
        self.state.lock().expect("state lock").budget.clone()
    }

    pub(super) fn failed_tracking(&self) -> Vec<(String, String)> {
        self.state
            .lock()
            .expect("state lock")
            .failed_tracking
            .clone()
    }

    pub(super) fn too_long(&self) -> Vec<Option<JsonMap>> {
        self.state.lock().expect("state lock").too_long.clone()
    }
}

#[async_trait]
impl SlackAlerting for FakeSlack {
    fn alerting_configured(&self) -> bool {
        self.configured
    }

    async fn failed_tracking_alert(&self, error_message: &str, failing_model: &str) {
        self.journal
            .record(format!("failed_tracking:{error_message}:{failing_model}"));
        self.state
            .lock()
            .expect("state lock")
            .failed_tracking
            .push((error_message.to_owned(), failing_model.to_owned()));
    }

    async fn budget_alerts(&self, kind: BudgetAlertKind, _user_info: &CallInfo) {
        self.journal.record(format!("budget:{kind:?}"));
        self.state.lock().expect("state lock").budget.push(kind);
    }

    async fn send_alert(&self, alert: SlackAlert<'_>) {
        self.journal.record(format!("send_alert:{}", alert.message));
        self.state
            .lock()
            .expect("state lock")
            .sent
            .push(SlackRecord {
                message: alert.message.to_owned(),
                level: alert.level,
                alert_type: alert.alert_type,
                alerting_metadata: alert.alerting_metadata.clone(),
                langfuse_trace: alert.langfuse_trace.map(str::to_owned),
            });
    }

    async fn response_taking_too_long(&self, data: Option<JsonMap>) {
        self.journal.record("response_taking_too_long".to_owned());
        self.state.lock().expect("state lock").too_long.push(data);
    }
}

#[derive(Default)]
pub(super) struct FakeEmail {
    budget: Mutex<Vec<BudgetAlertKind>>,
}

impl FakeEmail {
    pub(super) fn budget(&self) -> Vec<BudgetAlertKind> {
        self.budget.lock().expect("budget lock").clone()
    }
}

#[async_trait]
impl EmailAlerting for FakeEmail {
    async fn budget_alerts(&self, kind: BudgetAlertKind, _user_info: &CallInfo) {
        self.budget.lock().expect("budget lock").push(kind);
    }
}

#[derive(Clone)]
pub(super) struct ServiceRecord {
    pub service: FailureService,
    pub duration: Duration,
    pub error: String,
    pub call_type: String,
}

#[derive(Default)]
pub(super) struct FakeServiceLogging {
    records: Mutex<Vec<ServiceRecord>>,
}

impl FakeServiceLogging {
    pub(super) fn records(&self) -> Vec<ServiceRecord> {
        self.records.lock().expect("records lock").clone()
    }
}

#[async_trait]
impl ServiceLogging for FakeServiceLogging {
    async fn async_service_failure_hook(
        &self,
        service: FailureService,
        duration: Duration,
        error: &str,
        call_type: &str,
    ) {
        self.records
            .lock()
            .expect("records lock")
            .push(ServiceRecord {
                service,
                duration,
                error: error.to_owned(),
                call_type: call_type.to_owned(),
            });
    }
}

#[derive(Default)]
pub(super) struct FakeSentry {
    messages: Mutex<Vec<String>>,
    exceptions: Mutex<Vec<String>>,
}

impl FakeSentry {
    pub(super) fn messages(&self) -> Vec<String> {
        self.messages.lock().expect("messages lock").clone()
    }

    pub(super) fn exceptions(&self) -> Vec<String> {
        self.exceptions.lock().expect("exceptions lock").clone()
    }
}

impl SentryCapture for FakeSentry {
    fn capture_message(&self, message: &str) {
        self.messages
            .lock()
            .expect("messages lock")
            .push(message.to_owned());
    }

    fn capture_exception(&self, error: &ProxyHookError) {
        self.exceptions
            .lock()
            .expect("exceptions lock")
            .push(error.to_string());
    }
}

pub(super) struct FixedClock {
    now: DateTime<FixedOffset>,
}

impl FixedClock {
    pub(super) fn new() -> Self {
        Self {
            now: DateTime::parse_from_rfc3339("2026-01-01T12:34:56+00:00")
                .expect("fixed timestamp"),
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

pub(super) struct SteppingClock {
    state: Mutex<(DateTime<FixedOffset>, DateTime<FixedOffset>)>,
}

impl SteppingClock {
    pub(super) fn new(first: &str, next: &str) -> Self {
        Self {
            state: Mutex::new((
                DateTime::parse_from_rfc3339(first).expect("first timestamp"),
                DateTime::parse_from_rfc3339(next).expect("next timestamp"),
            )),
        }
    }

    pub(super) fn advance(&self) {
        let mut state = self.state.lock().expect("clock lock");
        state.0 = state.1;
    }
}

impl Clock for SteppingClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.state.lock().expect("clock lock").0
    }
}

pub(super) struct FakeCallback {
    journal: Journal,
    name: String,
    reply: Result<FailureReply, ProxyHookError>,
}

impl FakeCallback {
    pub(super) fn new(
        journal: &Journal,
        name: &str,
        reply: Result<FailureReply, ProxyHookError>,
    ) -> Arc<Self> {
        Arc::new(Self {
            journal: journal.clone(),
            name: name.to_owned(),
            reply,
        })
    }
}

#[async_trait]
impl ProxyCallback for FakeCallback {
    fn overrides(&self) -> HookOverrides {
        HookOverrides::default()
    }

    fn class_name(&self) -> &str {
        &self.name
    }

    fn is_base_logger(&self) -> bool {
        false
    }

    fn enforces_request_content(&self) -> bool {
        false
    }

    fn use_native_lifecycle_hooks(&self) -> bool {
        false
    }

    fn guardrail(&self) -> Option<&dyn GuardrailCallback> {
        None
    }

    async fn post_call_failure(
        &self,
        request_data: &mut JsonMap,
        _original_exception: &ProxyHookError,
        _user_api_key_dict: &UserAPIKeyAuth,
        traceback: Option<&str>,
    ) -> Result<FailureReply, ProxyHookError> {
        let keys: BTreeSet<&str> = request_data.keys().map(String::as_str).collect();
        self.journal
            .record(format!("callback:{}:{keys:?}:{traceback:?}", self.name));
        self.reply.clone()
    }
}

pub(super) fn auth() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).expect("auth")
}

pub(super) fn redactor() -> Arc<Redactor> {
    Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor"))
}

pub(super) fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("json")
}

pub(super) fn object(pairs: &[(&str, JsonValue)]) -> JsonMap {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) async fn drain(tasks: &TaskTracker) {
    tasks.close();
    tasks.wait().await;
}

pub(super) fn http_error(status: u16, detail: &str) -> ProxyHookError {
    ProxyHookError::Http {
        status,
        detail: detail.to_owned(),
    }
}

pub(super) fn other_error(message: &str) -> ProxyHookError {
    ProxyHookError::Other(message.to_owned())
}

pub(super) fn replace(status: u16, detail: &str) -> FailureReply {
    FailureReply::Replace(HttpException {
        status,
        detail: detail.to_owned(),
    })
}

pub(super) fn deployment(provider: Option<&str>, model: &str) -> ListedDeployment {
    ListedDeployment {
        provider: provider.map(str::to_owned),
        model_info: JsonMap::new(),
        model: model.to_owned(),
    }
}

pub(super) fn details() -> ModelCallDetails {
    ModelCallDetails {
        start_time: None,
        first_api_call_start_time: None,
        call_type: None,
        combined_usage_object: None,
        response_cost: None,
        standard_logging_object: None,
        model: None,
        messages: None,
        optional_params: None,
        no_upstream_llm_call: false,
        cache_hit: false,
    }
}

pub(super) fn call_info(alert_emails: Option<Vec<String>>) -> CallInfo {
    CallInfo {
        spend: Decimal::ZERO,
        max_budget: None,
        soft_budget: None,
        token: None,
        customer_id: None,
        user_id: None,
        team_id: None,
        team_alias: None,
        organization_id: None,
        user_email: None,
        key_alias: None,
        projected_exceeded_date: None,
        projected_spend: None,
        event_group: EntityType::Key,
        alert_emails,
        max_budget_alert_emails: None,
    }
}

pub(super) struct AlertParts {
    pub alerting: Option<Vec<AlertingClient>>,
    pub alert_types: Vec<AlertType>,
    pub proxy_base_url: Option<String>,
    pub alerting_threshold: Duration,
    pub email: Option<Arc<FakeEmail>>,
    pub sentry: Option<Arc<FakeSentry>>,
    pub redactor: Option<Arc<Redactor>>,
    pub clock: Option<Arc<dyn Clock>>,
}

impl AlertParts {
    pub(super) fn new(alerting: Option<Vec<AlertingClient>>, alert_types: Vec<AlertType>) -> Self {
        Self {
            alerting,
            alert_types,
            proxy_base_url: None,
            alerting_threshold: Duration::from_secs(300),
            email: None,
            sentry: None,
            redactor: None,
            clock: None,
        }
    }
}

fn email_port(email: Arc<FakeEmail>) -> Arc<dyn EmailAlerting> {
    email
}

fn sentry_port(sentry: Arc<FakeSentry>) -> Arc<dyn SentryCapture> {
    sentry
}

pub(super) fn proxy_alerts(
    parts: AlertParts,
    usage_cache: Arc<FakeUsageCache>,
    slack: Arc<FakeSlack>,
    service_logging: Arc<FakeServiceLogging>,
) -> Arc<ProxyAlerts> {
    Arc::new(ProxyAlerts {
        alerting: parts.alerting,
        alert_types: parts.alert_types,
        alerting_threshold: parts.alerting_threshold,
        proxy_base_url: parts.proxy_base_url,
        usage_cache,
        slack,
        email: parts.email.map(email_port),
        service_logging,
        sentry: parts.sentry.map(sentry_port),
        redactor: parts.redactor,
        clock: parts.clock.unwrap_or_else(|| Arc::new(FixedClock::new())),
        tasks: TaskTracker::new(),
    })
}

pub(super) struct HookParts {
    pub alerts: Arc<ProxyAlerts>,
    pub factory: Arc<FakeFactory>,
    pub counter: Arc<FakeCounter>,
    pub catalog: Arc<FakeCatalog>,
    pub routes: Arc<FakeRoutes>,
    pub log_client_error_tracebacks: bool,
    pub new_call_id: String,
}

pub(super) fn failure_hook(parts: HookParts) -> FailureHook {
    let new_call_id = parts.new_call_id;
    FailureHook {
        alerts: parts.alerts,
        logging_factory: parts.factory,
        token_counter: parts.counter,
        deployments: parts.catalog,
        route_call_types: parts.routes,
        new_call_id: Arc::new(move || new_call_id.clone()),
        log_client_error_tracebacks: parts.log_client_error_tracebacks,
    }
}
