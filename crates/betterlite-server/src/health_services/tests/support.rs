use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::health_services::Settings;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::alert_dispatch::AlertLevel;
use betterlite_logging::budget_email::event::WebhookEvent;
use betterlite_logging::exporter_options::health::{HealthState, IntegrationHealthStatus};
use betterlite_logging::registry::identity::CallbackIdentity;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::failure_hook::ports::Clock;
use crate::health_readiness::callback_listing::CallbackEntry;
use crate::health_services::alert_tests::{AlertActions, TeamsDelivery, UserBudgetAlert};
use crate::health_services::mock_requests::{MockRequest, MockRequests};
use crate::health_services::service_probes::{
    CallbackSource, IntegrationChecks, PointFiveSetupError, ServiceTestError,
};
use crate::health_services::service_routes::{HealthServicesState, health_services_routes};

pub(crate) const ADMIN_KEY: &str = r#"{"token": "hashed-admin-key", "key_alias": "admin-alias", "user_id": "admin-user", "team_id": "admin-team", "user_role": "proxy_admin"}"#;
pub(crate) const USER_KEY: &str = r#"{"token": "hashed-user-key", "key_alias": "user-alias", "user_id": "user-1", "team_id": "team-1", "user_role": "internal_user"}"#;

#[derive(Debug)]
pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).expect("the response body is the documented JSON")
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("the response body is UTF-8")
    }

    pub(crate) fn error(&self) -> ProxyErrorBody {
        self.json()
    }

    pub(crate) fn success(&self) -> SuccessReply {
        self.json()
    }

    pub(crate) fn slack(&self) -> SlackReply {
        self.json()
    }

    pub(crate) fn health(&self) -> HealthReply {
        self.json()
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProxyErrorBody {
    pub(crate) error: ErrorObjectBody,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorObjectBody {
    pub(crate) message: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) param: Option<String>,
    pub(crate) code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SuccessReply {
    pub(crate) status: String,
    pub(crate) message: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SlackReply {
    pub(crate) status: String,
    pub(crate) alert_types: Vec<String>,
    pub(crate) message: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HealthReply {
    pub(crate) status: String,
    pub(crate) message: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Answer {
    Healthy,
    HealthyWithText(&'static str),
    Unhealthy(&'static str),
    Failed(&'static str),
}

fn resolved(answer: Answer) -> Result<IntegrationHealthStatus, String> {
    match answer {
        Answer::Healthy => Ok(IntegrationHealthStatus::healthy()),
        Answer::HealthyWithText(text) => Ok(IntegrationHealthStatus {
            status: HealthState::Healthy,
            error_message: Some((*text).to_owned()),
        }),
        Answer::Unhealthy(message) => Ok(IntegrationHealthStatus::unhealthy((*message).to_owned())),
        Answer::Failed(message) => Err((*message).to_owned()),
    }
}

pub(crate) fn assert_mock_request(request: &MockRequest) {
    assert_eq!(request.model, "openai/litellm-mock-response-model");
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, "user");
    assert_eq!(request.messages[0].content, "Hey, how's it going?");
    assert_eq!(request.user, "litellm:/health/services");
    assert_eq!(request.mock_response, "This is a mock response");
}

#[derive(Default)]
pub(crate) struct FakeMock {
    seen: Mutex<Vec<MockRequest>>,
    failure: Option<String>,
}

impl FakeMock {
    pub(crate) fn failing(message: &str) -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            failure: Some(message.to_owned()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<MockRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl MockRequests for FakeMock {
    async fn send_mock_request(&self, request: &MockRequest) -> Result<(), ServiceTestError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(*request);
        self.failure.as_ref().map_or(Ok(()), |message| {
            Err(ServiceTestError {
                message: message.clone(),
            })
        })
    }
}

pub(crate) struct FakeChecks {
    datadog: Result<IntegrationHealthStatus, String>,
    datadog_metrics: Result<IntegrationHealthStatus, String>,
    arize: Result<IntegrationHealthStatus, String>,
    galileo: Result<IntegrationHealthStatus, String>,
    newrelic: Result<IntegrationHealthStatus, String>,
    sqs: Result<IntegrationHealthStatus, String>,
    pointfive: Result<IntegrationHealthStatus, String>,
    langfuse: Result<Option<String>, String>,
}

impl Default for FakeChecks {
    fn default() -> Self {
        Self {
            datadog: Ok(IntegrationHealthStatus::healthy()),
            datadog_metrics: Ok(IntegrationHealthStatus::healthy()),
            arize: Ok(IntegrationHealthStatus::healthy()),
            galileo: Ok(IntegrationHealthStatus::healthy()),
            newrelic: Ok(IntegrationHealthStatus::healthy()),
            sqs: Ok(IntegrationHealthStatus::healthy()),
            pointfive: Ok(IntegrationHealthStatus::healthy()),
            langfuse: Ok(None),
        }
    }
}

impl FakeChecks {
    pub(crate) fn with_datadog(mut self, answer: Answer) -> Self {
        self.datadog = resolved(answer);
        self
    }

    pub(crate) fn with_datadog_metrics(mut self, answer: Answer) -> Self {
        self.datadog_metrics = resolved(answer);
        self
    }

    pub(crate) fn with_arize(mut self, answer: Answer) -> Self {
        self.arize = resolved(answer);
        self
    }

    pub(crate) fn with_galileo(mut self, answer: Answer) -> Self {
        self.galileo = resolved(answer);
        self
    }

    pub(crate) fn with_newrelic(mut self, answer: Answer) -> Self {
        self.newrelic = resolved(answer);
        self
    }

    pub(crate) fn with_sqs(mut self, answer: Answer) -> Self {
        self.sqs = resolved(answer);
        self
    }

    pub(crate) fn with_pointfive(mut self, answer: Answer) -> Self {
        self.pointfive = resolved(answer);
        self
    }

    pub(crate) fn with_langfuse(mut self, reason: Option<&str>) -> Self {
        self.langfuse = Ok(reason.map(str::to_owned));
        self
    }
}

#[async_trait]
impl IntegrationChecks for FakeChecks {
    async fn datadog(&self) -> Result<IntegrationHealthStatus, ServiceTestError> {
        self.datadog
            .clone()
            .map_err(|message| ServiceTestError { message })
    }

    async fn datadog_metrics(&self) -> Result<IntegrationHealthStatus, ServiceTestError> {
        self.datadog_metrics
            .clone()
            .map_err(|message| ServiceTestError { message })
    }

    async fn arize(&self) -> Result<IntegrationHealthStatus, ServiceTestError> {
        self.arize
            .clone()
            .map_err(|message| ServiceTestError { message })
    }

    async fn galileo(&self) -> Result<IntegrationHealthStatus, ServiceTestError> {
        self.galileo
            .clone()
            .map_err(|message| ServiceTestError { message })
    }

    async fn newrelic(&self) -> Result<IntegrationHealthStatus, ServiceTestError> {
        self.newrelic
            .clone()
            .map_err(|message| ServiceTestError { message })
    }

    async fn sqs(&self) -> Result<IntegrationHealthStatus, ServiceTestError> {
        self.sqs
            .clone()
            .map_err(|message| ServiceTestError { message })
    }

    async fn pointfive(&self) -> Result<IntegrationHealthStatus, PointFiveSetupError> {
        self.pointfive
            .clone()
            .map_err(|message| PointFiveSetupError { message })
    }

    async fn langfuse_auth_check(&self) -> Result<Option<String>, ServiceTestError> {
        match &self.langfuse {
            Ok(reason) => Ok(reason.clone()),
            Err(message) => Err(ServiceTestError {
                message: message.clone(),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SentAlert {
    pub(crate) message: String,
    pub(crate) level: AlertLevel,
    pub(crate) alert_type: AlertType,
}

#[derive(Default)]
pub(crate) struct FakeAlerts {
    webhook_alert_types: Option<Vec<AlertType>>,
    active_alert_types: Option<Vec<AlertType>>,
    teams_url: Option<SecretString>,
    teams_delivery: Option<TeamsDelivery>,
    has_database: bool,
    email_sent: bool,
    budget_tests: Mutex<Vec<UserBudgetAlert>>,
    sent_alerts: Mutex<Vec<SentAlert>>,
    emails: Mutex<Vec<WebhookEvent>>,
    teams_posts: Mutex<Vec<(String, String)>>,
    reports: Mutex<Vec<&'static str>>,
}

impl FakeAlerts {
    pub(crate) fn with_webhook_alert_types(mut self, alert_types: &[AlertType]) -> Self {
        self.webhook_alert_types = Some(alert_types.to_vec());
        self
    }

    pub(crate) fn with_active_alert_types(mut self, alert_types: &[AlertType]) -> Self {
        self.active_alert_types = Some(alert_types.to_vec());
        self
    }

    pub(crate) fn with_teams_url(mut self, url: Option<&str>) -> Self {
        self.teams_url = url.map(|text| SecretString::from(text.to_owned()));
        self
    }

    pub(crate) fn with_teams_delivery(mut self, delivery: TeamsDelivery) -> Self {
        self.teams_delivery = Some(delivery);
        self
    }

    pub(crate) fn with_database(mut self, has_database: bool) -> Self {
        self.has_database = has_database;
        self
    }

    pub(crate) fn with_email_sent(mut self, sent: bool) -> Self {
        self.email_sent = sent;
        self
    }

    pub(crate) fn budget_tests(&self) -> Vec<UserBudgetAlert> {
        self.budget_tests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn sent_alerts(&self) -> Vec<SentAlert> {
        self.sent_alerts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn emails(&self) -> Vec<WebhookEvent> {
        self.emails
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn teams_posts(&self) -> Vec<(String, String)> {
        self.teams_posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn reports(&self) -> Vec<&'static str> {
        self.reports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl AlertActions for FakeAlerts {
    async fn send_user_budget_test(&self, alert: &UserBudgetAlert) -> Result<(), ServiceTestError> {
        self.budget_tests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(alert.clone());
        Ok(())
    }

    async fn send_alert(
        &self,
        message: &str,
        level: AlertLevel,
        alert_type: AlertType,
    ) -> Result<(), ServiceTestError> {
        self.sent_alerts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SentAlert {
                message: message.to_owned(),
                level,
                alert_type,
            });
        Ok(())
    }

    async fn send_test_email(&self, event: &WebhookEvent) -> Result<bool, ServiceTestError> {
        self.emails
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.clone());
        Ok(self.email_sent)
    }

    async fn post_teams(
        &self,
        url: &SecretString,
        text: &str,
    ) -> Result<TeamsDelivery, ServiceTestError> {
        self.teams_posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.expose_secret().to_owned(), text.to_owned()));
        self.teams_delivery.as_ref().map_or_else(
            || {
                Ok(TeamsDelivery {
                    status_code: 200,
                    body: "ok".to_owned(),
                })
            },
            |delivery| Ok(delivery.clone()),
        )
    }

    fn webhook_alert_types(&self) -> Option<Vec<AlertType>> {
        self.webhook_alert_types.clone()
    }

    fn active_alert_types(&self) -> Option<Vec<AlertType>> {
        self.active_alert_types.clone()
    }

    fn ms_teams_webhook_url(&self) -> Option<SecretString> {
        self.teams_url.clone()
    }

    fn has_database(&self) -> bool {
        self.has_database
    }

    fn send_monthly_spend_report(&self) {
        self.reports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("monthly");
    }

    fn send_weekly_spend_report(&self) {
        self.reports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("weekly");
    }
}

#[derive(Default)]
pub(crate) struct FakeCallbacks {
    pub(crate) entries: Vec<CallbackEntry>,
}

impl FakeCallbacks {
    pub(crate) fn plain(names: &[&str]) -> Self {
        Self {
            entries: names
                .iter()
                .map(|name| CallbackEntry::Plain((*name).to_owned()))
                .collect(),
        }
    }

    pub(crate) fn registered(class_name: &str) -> Self {
        Self {
            entries: vec![CallbackEntry::Registered(CallbackIdentity {
                class_name: class_name.to_owned(),
                attributes: IndexMap::new(),
            })],
        }
    }
}

impl CallbackSource for FakeCallbacks {
    fn success_callback_entries(&self) -> Vec<CallbackEntry> {
        self.entries.clone()
    }
}

pub(crate) struct FakeClock {
    pub(crate) now: DateTime<FixedOffset>,
}

impl Default for FakeClock {
    fn default() -> Self {
        Self {
            now: DateTime::parse_from_rfc3339("2026-01-02T03:04:05+00:00")
                .expect("the fixed clock parses"),
        }
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

#[derive(Default)]
pub(crate) struct World {
    pub(crate) mock: Arc<FakeMock>,
    pub(crate) checks: Arc<FakeChecks>,
    pub(crate) alerts: Arc<FakeAlerts>,
    pub(crate) callbacks: Arc<FakeCallbacks>,
    pub(crate) clock: Arc<FakeClock>,
    pub(crate) settings: Settings,
    pub(crate) alerting: Vec<String>,
}

impl World {
    pub(crate) fn with_mock(mut self, mock: FakeMock) -> Self {
        self.mock = Arc::new(mock);
        self
    }

    pub(crate) fn with_checks(mut self, checks: FakeChecks) -> Self {
        self.checks = Arc::new(checks);
        self
    }

    pub(crate) fn with_alerts(mut self, alerts: FakeAlerts) -> Self {
        self.alerts = Arc::new(alerts);
        self
    }

    pub(crate) fn with_callbacks(mut self, callbacks: FakeCallbacks) -> Self {
        self.callbacks = Arc::new(callbacks);
        self
    }

    pub(crate) fn with_alerting(mut self, alerting: &[&str]) -> Self {
        self.alerting = alerting.iter().map(|entry| (*entry).to_owned()).collect();
        self
    }

    pub(crate) fn with_test_email(mut self, address: &str) -> Self {
        self.settings.test_email_address = Some(address.to_owned());
        self
    }

    pub(crate) fn router(&self) -> Router {
        let state = HealthServicesState {
            mock: Arc::<FakeMock>::clone(&self.mock),
            checks: Arc::<FakeChecks>::clone(&self.checks),
            alerts: Arc::<FakeAlerts>::clone(&self.alerts),
            callbacks: Arc::<FakeCallbacks>::clone(&self.callbacks),
            clock: Arc::<FakeClock>::clone(&self.clock),
            settings: self.settings.clone(),
            alerting: self.alerting.clone(),
        };
        health_services_routes(state)
    }

    pub(crate) async fn get(&self, uri: &str, caller: Option<&AuthContext>) -> Reply {
        get(&self.router(), uri, caller).await
    }
}

pub(crate) async fn get(router: &Router, uri: &str, caller: Option<&AuthContext>) -> Reply {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .expect("the request builds");
    if let Some(identity) = caller {
        request.extensions_mut().insert(identity.clone());
    }
    let response = Router::clone(router)
        .oneshot(request)
        .await
        .expect("the route answers");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads");
    Reply {
        status,
        body: body.to_vec(),
    }
}

pub(crate) fn context(subject: &str, roles: &[Role], source_key: SourceKey) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: roles.to_vec(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key,
        },
    }
}

pub(crate) fn master_context() -> AuthContext {
    master_caller()
}

pub(crate) fn viewer_context() -> AuthContext {
    context("viewer", &[Role::PlatformViewer], SourceKey(None))
}

pub(crate) fn user_context() -> AuthContext {
    context("user", &[], SourceKey(None))
}

pub(crate) fn key_context(key_fields: &str) -> AuthContext {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(key_fields).expect("the key fixture parses");
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture builds");
    context("key-owner", &[], SourceKey(Some(key)))
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the captured log lines are UTF-8")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
