use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use betterlite_config::sections::audit_events::Settings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::registry::phases::CallbackSelection;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use tower::ServiceExt as _;

use crate::audit_events::callbacks::{
    AuditCallbackDispatcher, AuditCallbackError, AuditCallbackFuture, AuditCallbackRegistry,
    AuditLogCallback, AuditTaskSpawner,
};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::ConfiguredAuditSettings;
use crate::audit_events::sink::{
    AuditClock, AuditEventSink, AuditLogRepository, AuditRepositoryError,
};
use crate::auth::auth_identity::AuthContext;
use crate::cache_settings_update::route::StoreModelInDbSettings;
use crate::callback_config::ports::{
    AlertingState, CallbackClock, ConfigSaver, RouterSettingsReader, RuntimeCallback,
    RuntimeCallbacks,
};
use crate::callback_config::{CallbackConfigState, routes};
use crate::config_overlay_port::{ConfigOverlayPort, ConfigOverlayPortError};
use crate::deployment_sync_port::DeploymentSyncPort;
use crate::proxy_config_state::error::ConfigStateError;
use crate::proxy_config_state::snapshot::ConfigWithBaseline;

pub(crate) const MOMENT: &str = "2026-01-02T03:04:05.123456+02:00";

pub(crate) fn json(source: &str) -> JsonValue {
    serde_json::from_str(source).expect("valid JSON")
}

pub(crate) fn object(pairs: &[(&str, JsonValue)]) -> BTreeMap<String, JsonValue> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(MOMENT).expect("a valid timestamp")
}

pub(crate) fn principal(subject: &str) -> Principal {
    Principal {
        principal_type: PrincipalType::ServiceAccount,
        subject: subject.to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}

pub(crate) fn master_key() -> Principal {
    principal("master_key")
}

pub(crate) fn internal_key() -> Principal {
    principal("key-1")
}

pub(crate) fn webhook(url: &str) -> WebhookUrl {
    WebhookUrl::Single(SecretString::from(url.to_owned()))
}

pub(crate) struct ScriptedOverlay {
    pub(crate) reloads: Arc<AtomicUsize>,
    pub(crate) config: ConfigWithBaseline,
    pub(crate) failure: Option<ConfigOverlayPortError>,
}

#[async_trait]
impl ConfigOverlayPort for ScriptedOverlay {
    async fn reload_config(&self) -> Result<ConfigWithBaseline, ConfigOverlayPortError> {
        self.reloads.fetch_add(1, Ordering::SeqCst);
        self.failure.map_or_else(|| Ok(self.config.clone()), Err)
    }

    async fn serve_pass_through_endpoints(
        &self,
        _db_endpoints: &[JsonValue],
    ) -> Result<(), ConfigOverlayPortError> {
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct RecordingSaver {
    pub(crate) saves: Mutex<Vec<ConfigWithBaseline>>,
    pub(crate) failures: Mutex<Vec<ConfigStateError>>,
}

#[async_trait]
impl ConfigSaver for RecordingSaver {
    async fn save_config(&self, config: ConfigWithBaseline) -> Result<(), ConfigStateError> {
        self.saves
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(config);
        let failure = self
            .failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        failure.map_or_else(|| Ok(()), Err)
    }
}

#[derive(Default)]
pub(crate) struct FakeCallbacks {
    pub(crate) selection: CallbackSelection,
    pub(crate) objects: Vec<RuntimeCallback>,
    pub(crate) classes: BTreeMap<String, String>,
}

impl RuntimeCallbacks for FakeCallbacks {
    fn selection(&self) -> CallbackSelection {
        self.selection.clone()
    }

    fn objects(&self) -> Vec<RuntimeCallback> {
        self.objects.clone()
    }

    fn class_str_for_name(&self, name: &str) -> Option<String> {
        self.classes.get(name).cloned()
    }
}

pub(crate) struct FakeAlerting {
    pub(crate) alert_types: Vec<AlertType>,
    pub(crate) webhooks: BTreeMap<AlertType, WebhookUrl>,
}

impl AlertingState for FakeAlerting {
    fn active_alert_types(&self) -> Vec<AlertType> {
        self.alert_types.clone()
    }

    fn alert_to_webhook_url(&self) -> BTreeMap<AlertType, WebhookUrl> {
        self.webhooks.clone()
    }
}

pub(crate) struct FakeRouter {
    pub(crate) settings: BTreeMap<String, JsonValue>,
}

impl RouterSettingsReader for FakeRouter {
    fn get_settings(&self) -> BTreeMap<String, JsonValue> {
        self.settings.clone()
    }
}

#[derive(Default)]
pub(crate) struct FakeDeployments {
    pub(crate) calls: AtomicUsize,
}

impl DeploymentSyncPort for FakeDeployments {
    fn add_deployment(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
        })
    }
}

pub(crate) struct FixedCallbackClock {
    pub(crate) moment: DateTime<FixedOffset>,
}

impl CallbackClock for FixedCallbackClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.moment
    }
}

pub(crate) struct StoreFlag(pub(crate) bool);

impl StoreModelInDbSettings for StoreFlag {
    fn store_model_in_db(&self) -> bool {
        self.0
    }
}

#[derive(Default)]
pub(crate) struct RecordingRepository {
    pub(crate) inserted: Mutex<Vec<AuditLogRecord>>,
}

#[async_trait]
impl AuditLogRepository for RecordingRepository {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        self.inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

pub(crate) struct NoCallbacks;

impl AuditCallbackRegistry for NoCallbacks {
    fn resolve(&self, _name: &str) -> Option<Arc<dyn AuditLogCallback>> {
        None
    }

    fn build_dedicated_s3_logger(
        &self,
        _params: &JsonValue,
    ) -> Result<Arc<dyn AuditLogCallback>, AuditCallbackError> {
        Err(AuditCallbackError::Failed)
    }
}

pub(crate) struct DroppingSpawner;

impl AuditTaskSpawner for DroppingSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        drop(task);
    }
}

pub(crate) struct FixedAuditClock {
    pub(crate) moment: DateTime<FixedOffset>,
}

impl AuditClock for FixedAuditClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.moment
    }
}

pub(crate) fn enabled_sink(repository: Arc<RecordingRepository>) -> AuditEventSink {
    AuditEventSink {
        settings: Arc::new(ConfiguredAuditSettings {
            config: Settings {
                store_audit_logs: Some(true),
                store_audit_logs_env: None,
                audit_log_callbacks: Vec::new(),
                s3_audit_callback_params: None,
            },
            callbacks: Vec::new(),
        }),
        callbacks: Arc::new(AuditCallbackDispatcher::new(
            Arc::new(NoCallbacks),
            Arc::new(DroppingSpawner),
        )),
        repository,
        clock: Arc::new(FixedAuditClock { moment: moment() }),
        new_id: Arc::new(|| "audit-id-1".to_owned()),
    }
}

pub(crate) struct Harness {
    pub(crate) config: BTreeMap<String, JsonValue>,
    pub(crate) overlay_failure: Option<ConfigOverlayPortError>,
    pub(crate) reloads: Arc<AtomicUsize>,
    pub(crate) saver: Arc<RecordingSaver>,
    pub(crate) database_connected: bool,
    pub(crate) store_model_in_db: bool,
    pub(crate) callbacks: FakeCallbacks,
    pub(crate) alert_types: Vec<AlertType>,
    pub(crate) webhooks: BTreeMap<AlertType, WebhookUrl>,
    pub(crate) router: Option<Arc<dyn RouterSettingsReader>>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) clock: DateTime<FixedOffset>,
    pub(crate) audit_repository: Arc<RecordingRepository>,
    pub(crate) deployments: Arc<FakeDeployments>,
}

impl Harness {
    pub(crate) fn new(config: BTreeMap<String, JsonValue>) -> Self {
        Self {
            config,
            overlay_failure: None,
            reloads: Arc::new(AtomicUsize::new(0)),
            saver: Arc::new(RecordingSaver::default()),
            database_connected: true,
            store_model_in_db: true,
            callbacks: FakeCallbacks::default(),
            alert_types: Vec::new(),
            webhooks: BTreeMap::new(),
            router: None,
            environment: BTreeMap::new(),
            clock: moment(),
            audit_repository: Arc::new(RecordingRepository::default()),
            deployments: Arc::new(FakeDeployments::default()),
        }
    }

    pub(crate) fn with_environment(mut self, pairs: &[(&str, &str)]) -> Self {
        self.environment = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        self
    }

    pub(crate) fn with_alerting(
        mut self,
        alert_types: Vec<AlertType>,
        webhooks: BTreeMap<AlertType, WebhookUrl>,
    ) -> Self {
        self.alert_types = alert_types;
        self.webhooks = webhooks;
        self
    }

    pub(crate) fn with_router(mut self, settings: BTreeMap<String, JsonValue>) -> Self {
        self.router = Some(router_port(Arc::new(FakeRouter { settings })));
        self
    }

    pub(crate) fn without_database(mut self) -> Self {
        self.database_connected = false;
        self
    }

    pub(crate) fn without_store_model_in_db(mut self) -> Self {
        self.store_model_in_db = false;
        self
    }

    pub(crate) fn failing_save(&self, error: ConfigStateError) -> &Self {
        self.saver
            .failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(error);
        self
    }

    pub(crate) fn failing_reload(mut self, error: ConfigOverlayPortError) -> Self {
        self.overlay_failure = Some(error);
        self
    }

    pub(crate) fn with_clock(mut self, clock: DateTime<FixedOffset>) -> Self {
        self.clock = clock;
        self
    }

    pub(crate) fn saved(&self) -> Vec<ConfigWithBaseline> {
        self.saver
            .saves
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn audit_records(&self) -> Vec<AuditLogRecord> {
        self.audit_repository
            .inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn deployment_calls(&self) -> usize {
        self.deployments.calls.load(Ordering::SeqCst)
    }

    pub(crate) fn reload_calls(&self) -> usize {
        self.reloads.load(Ordering::SeqCst)
    }

    pub(crate) fn state(&self) -> CallbackConfigState {
        CallbackConfigState {
            overlay: Arc::new(self.overlay_source()),
            saver: saver_port(Arc::clone(&self.saver)),
            database_connected: self.database_connected,
            store_model_in_db: Arc::new(StoreFlag(self.store_model_in_db)),
            callbacks: Arc::new(self.callback_fakes()),
            alerting: Arc::new(self.alerting_state()),
            router: self.router.clone(),
            environment: Arc::new(self.environment.clone()),
            audit: Arc::new(enabled_sink(Arc::clone(&self.audit_repository))),
            clock: Arc::new(FixedCallbackClock { moment: self.clock }),
        }
    }

    fn overlay_source(&self) -> ScriptedOverlay {
        ScriptedOverlay {
            reloads: Arc::clone(&self.reloads),
            config: ConfigWithBaseline::with_baseline(self.config.clone(), self.config.clone()),
            failure: self.overlay_failure,
        }
    }

    fn callback_fakes(&self) -> FakeCallbacks {
        FakeCallbacks {
            selection: self.callbacks.selection.clone(),
            objects: self.callbacks.objects.clone(),
            classes: self.callbacks.classes.clone(),
        }
    }

    fn alerting_state(&self) -> FakeAlerting {
        FakeAlerting {
            alert_types: self.alert_types.clone(),
            webhooks: self.webhooks.clone(),
        }
    }

    pub(crate) async fn call(
        &self,
        request: Request<Body>,
        caller: Option<Principal>,
    ) -> (StatusCode, JsonValue) {
        let mut router = routes(
            self.state(),
            deployments_port(Arc::clone(&self.deployments)),
        );
        if let Some(principal) = caller {
            router = router.layer(Extension(AuthContext { principal }));
        }
        let response = router.oneshot(request).await.expect("the router answers");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body reads");
        let body = serde_json::from_slice(&bytes).expect("the body is JSON");
        (status, body)
    }
}

fn saver_port(saver: Arc<RecordingSaver>) -> Arc<dyn ConfigSaver> {
    saver
}

fn deployments_port(deployments: Arc<FakeDeployments>) -> Arc<dyn DeploymentSyncPort> {
    deployments
}

fn router_port(router: Arc<FakeRouter>) -> Arc<dyn RouterSettingsReader> {
    router
}

pub(crate) fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("a valid request")
}

pub(crate) fn request_with_body(method: &str, uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .expect("a valid request")
}
