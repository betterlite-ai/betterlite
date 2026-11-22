use std::collections::{BTreeMap, BTreeSet};
use std::future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_config::sections::key_event_sinks::Settings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_logging::budget_email::event::WebhookEvent;
use betterlite_secret_managers::secret_contracts::options::{
    DeleteOptions, SecretOptions, SecretRotation, SecretWrite,
};
use betterlite_secret_managers::secret_contracts::{
    BaseSecretManager, RotationError, SecretFuture,
};
use chrono::{DateTime, FixedOffset};
use secrecy::{ExposeSecret as _, SecretString};
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::audit_events::callbacks::{
    AuditCallbackConfig, AuditCallbackDispatcher, AuditCallbackError, AuditCallbackFuture,
    AuditCallbackRegistry, AuditLogCallback, AuditTaskSpawner,
};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::AuditSettings;
use crate::audit_events::sink::{
    AuditClock, AuditEventSink, AuditLogRepository, AuditRepositoryError,
};
use crate::key_event_sinks::email::CreatedKey;
use crate::key_event_sinks::error::EmailError;
use crate::key_event_sinks::ports::{KeyCreatedMailer, TeamDirectory};
use crate::key_event_sinks::secrets::SecretSink;
use crate::key_event_sinks::team_params::TeamSecretSettings;
use crate::key_events::KeyEvents;
use crate::key_events::audit::{KeyAuditScheduler, KeyEventAudit, SinkScheduler};
use crate::key_events::effects::{KeyEmailError, KeyEventEffects, KeyEventEmail};
use crate::key_events::error::KeyEventError;
use crate::key_events::types::{KeyEvent, KeyEventCaller};

pub(super) const NOW: &str = "2026-01-02T03:04:05+00:00";
pub(super) const PREFIX: &str = "litellm/";

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// every collaborator appends its step here, so a test reads the whole order of one dispatch
#[derive(Default)]
pub(super) struct Board {
    pub(super) steps: Mutex<Vec<String>>,
}

impl Board {
    fn mark(&self, step: String) {
        lock(&self.steps).push(step);
    }

    pub(super) fn steps(&self) -> Vec<String> {
        lock(&self.steps).clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Call {
    Read {
        name: String,
    },
    Write {
        name: String,
        value: String,
    },
    Delete {
        name: String,
    },
    Rotate {
        current: String,
        new: String,
        value: String,
    },
}

impl Call {
    fn label(&self) -> String {
        match self {
            Self::Read { name } => format!("read:{name}"),
            Self::Write { name, .. } => format!("write:{name}"),
            Self::Delete { name } => format!("delete:{name}"),
            Self::Rotate { current, new, .. } => format!("rotate:{current}>{new}"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted secret manager refused {name}")]
pub(super) struct Refused {
    name: String,
}

pub(super) struct FakeManager {
    pub(super) board: Arc<Board>,
    pub(super) calls: Mutex<Vec<Call>>,
    pub(super) secrets: BTreeMap<String, String>,
    pub(super) failing: BTreeSet<String>,
}

impl FakeManager {
    fn answer(&self, call: Call) -> Result<(), Refused> {
        self.board.mark(call.label());
        let name = match &call {
            Call::Read { name } | Call::Write { name, .. } | Call::Delete { name } => name.clone(),
            Call::Rotate { current, .. } => current.clone(),
        };
        lock(&self.calls).push(call);
        if self.failing.contains(&name) {
            return Err(Refused { name });
        }
        Ok(())
    }
}

impl BaseSecretManager for FakeManager {
    type Params = TeamSecretSettings;
    type Response = ();
    type Error = Refused;

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: SecretOptions<'borrowed, TeamSecretSettings>,
    ) -> SecretFuture<'borrowed, Option<SecretString>, Refused> {
        Box::pin(future::ready(self.sync_read_secret(secret_name, options)))
    }

    fn sync_read_secret(
        &self,
        secret_name: &str,
        _options: SecretOptions<'_, TeamSecretSettings>,
    ) -> Result<Option<SecretString>, Refused> {
        self.answer(Call::Read {
            name: secret_name.to_owned(),
        })?;
        Ok(self
            .secrets
            .get(secret_name)
            .map(|value| SecretString::from(value.as_str())))
    }

    fn async_write_secret(
        &self,
        write: SecretWrite<'_, TeamSecretSettings>,
    ) -> SecretFuture<'_, (), Refused> {
        let name = write.secret_name.to_owned();
        let value = write.secret_value.expose_secret().to_owned();
        Box::pin(future::ready(self.answer(Call::Write { name, value })))
    }

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        _options: DeleteOptions<'borrowed, TeamSecretSettings>,
    ) -> SecretFuture<'borrowed, (), Refused> {
        Box::pin(future::ready(self.answer(Call::Delete {
            name: secret_name.to_owned(),
        })))
    }

    fn async_rotate_secret(
        &self,
        rotation: SecretRotation<'_, TeamSecretSettings>,
    ) -> SecretFuture<'_, (), RotationError<Refused>> {
        let call = Call::Rotate {
            current: rotation.current_secret_name.to_owned(),
            new: rotation.new_secret_name.to_owned(),
            value: rotation.new_secret_value.expose_secret().to_owned(),
        };
        let answer = self.answer(call);
        let settled = answer.map_err(|source| RotationError::Operation { source });
        Box::pin(future::ready(settled))
    }
}

pub(super) struct RecordingScheduler {
    pub(super) board: Arc<Board>,
    pub(super) records: Mutex<Vec<AuditLogRecord>>,
}

impl KeyAuditScheduler for RecordingScheduler {
    fn schedule(&self, record: AuditLogRecord) {
        self.board.mark(format!("audit:{}", record.object_id));
        lock(&self.records).push(record);
    }
}

// holds the sink futures until a test drains them, so nothing depends on a task racing the asserts
#[derive(Default)]
pub(super) struct CollectedSpawner {
    pub(super) tasks: Mutex<Vec<AuditCallbackFuture>>,
}

impl AuditTaskSpawner for CollectedSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        lock(&self.tasks).push(task);
    }
}

impl CollectedSpawner {
    pub(super) async fn drain(&self) {
        let tasks = std::mem::take(&mut *lock(&self.tasks));
        for task in tasks {
            task.await;
        }
    }
}

// records which team's secret-manager settings the sink looked up; no metadata keeps the params empty
pub(super) struct ScriptedTeams {
    pub(super) board: Arc<Board>,
    pub(super) lookups: Mutex<Vec<String>>,
}

#[async_trait]
impl TeamDirectory for ScriptedTeams {
    async fn team(
        &self,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamLookupError> {
        self.board.mark(format!("team:{team_id}"));
        lock(&self.lookups).push(team_id.to_owned());
        Ok(None)
    }
}

// the sink writes here, so a test reads the records that went through B2330's whole construction
#[derive(Default)]
pub(super) struct RecordingRepository {
    pub(super) records: Mutex<Vec<AuditLogRecord>>,
}

#[async_trait]
impl AuditLogRepository for RecordingRepository {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        lock(&self.records).push(record.clone());
        Ok(())
    }
}

// no callback is configured in these cases, so the dispatcher never asks the registry for one
pub(super) struct NoRegistry;

impl AuditCallbackRegistry for NoRegistry {
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

pub(super) struct TestAuditSettings {
    pub(super) enabled: bool,
}

impl AuditSettings for TestAuditSettings {
    fn is_audit_logging_enabled(&self, explicit: Option<bool>) -> bool {
        explicit.unwrap_or(self.enabled)
    }

    fn audit_log_callbacks(&self) -> &[AuditCallbackConfig] {
        &[]
    }

    fn s3_audit_callback_params(&self) -> Option<&JsonValue> {
        None
    }
}

pub(super) struct FixedClock;

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(NOW).unwrap()
    }
}

pub(super) struct RecordingEmail {
    pub(super) board: Arc<Board>,
    pub(super) created: Mutex<Vec<CreatedKey>>,
    pub(super) rotated: Mutex<Vec<(String, Option<String>)>>,
    pub(super) fail_created: AtomicBool,
    pub(super) fail_rotated: AtomicBool,
}

#[async_trait]
impl KeyEventEmail for RecordingEmail {
    async fn send_created(&self, response: CreatedKey) -> Result<(), KeyEmailError> {
        self.board.mark("email:created".to_owned());
        lock(&self.created).push(response);
        if self.fail_created.load(Ordering::Relaxed) {
            return Err(KeyEmailError::Rejected(EmailError::NullSpend));
        }
        Ok(())
    }

    async fn send_rotated(
        &self,
        response: &str,
        existing_key_alias: Option<&str>,
    ) -> Result<(), KeyEmailError> {
        self.board.mark("email:rotated".to_owned());
        lock(&self.rotated).push((response.to_owned(), existing_key_alias.map(str::to_owned)));
        if !self.fail_rotated.load(Ordering::Relaxed) {
            return Ok(());
        }
        Err(KeyEmailError::Rejected(EmailError::NullSpend))
    }
}

#[derive(Default)]
pub(super) struct RecordingMailer {
    pub(super) events: Mutex<Vec<WebhookEvent>>,
}

#[async_trait]
impl KeyCreatedMailer for RecordingMailer {
    async fn send(&self, event: WebhookEvent) {
        lock(&self.events).push(event);
    }
}

// one subscriber layer per test; a case reads the levels and texts it selected with a target prefix
#[derive(Clone)]
pub(super) struct Lines {
    pub(super) lines: Arc<Mutex<Vec<(Level, String)>>>,
    pub(super) target: &'static str,
    pub(super) level: Option<Level>,
}

impl Lines {
    pub(super) fn at(target: &'static str, level: Option<Level>) -> Self {
        Self {
            lines: Arc::new(Mutex::new(Vec::new())),
            target,
            level,
        }
    }

    pub(super) fn lines(&self) -> Vec<(Level, String)> {
        lock(&self.lines).clone()
    }
}

impl<S: Subscriber> Layer<S> for Lines {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        if !metadata.target().starts_with(self.target) {
            return;
        }
        if self.level.is_some_and(|level| *metadata.level() != level) {
            return;
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        lock(&self.lines).push((*metadata.level(), message.0));
    }
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

pub(super) struct Harness {
    pub(super) board: Arc<Board>,
    pub(super) manager: FakeManager,
    pub(super) scheduler: Arc<RecordingScheduler>,
    pub(super) email: RecordingEmail,
    pub(super) settings: Settings,
    pub(super) audit: KeyEventAudit,
    pub(super) logs: Arc<Lines>,
    pub(super) teams: Option<ScriptedTeams>,
    pub(super) client: bool,
    pub(super) _logging: DefaultGuard,
}

// which effects a case wires, so a case can leave the audit or the secret manager out
#[derive(Clone, Copy)]
pub(super) struct Wiring {
    pub(super) audit: bool,
    pub(super) client: bool,
}

pub(super) const AUDIT: Wiring = Wiring {
    audit: true,
    client: false,
};
pub(super) const STORE: Wiring = Wiring {
    audit: false,
    client: true,
};
pub(super) const BOTH: Wiring = Wiring {
    audit: true,
    client: true,
};

pub(super) fn harness() -> Harness {
    wired(STORE)
}

pub(super) fn wired(wiring: Wiring) -> Harness {
    let board = Arc::new(Board::default());
    let scheduler = Arc::new(RecordingScheduler {
        board: Arc::clone(&board),
        records: Mutex::new(Vec::new()),
    });
    let logs = Arc::new(Lines::at(
        "betterlite_server::key_events",
        Some(Level::WARN),
    ));
    let logging = set_default(tracing_subscriber::registry().with(Lines::clone(&logs)));
    Harness {
        manager: fake_manager(&board),
        email: recording_email(&board),
        settings: settings(),
        audit: key_audit(wiring.audit, &scheduler),
        scheduler,
        logs,
        board,
        teams: None,
        client: wiring.client,
        _logging: logging,
    }
}

fn settings() -> Settings {
    Settings {
        store_virtual_keys: true,
        prefix_for_stored_virtual_keys: PREFIX.to_owned(),
        description: None,
        tags: None,
        alerting: Vec::new(),
    }
}

fn fake_manager(board: &Arc<Board>) -> FakeManager {
    FakeManager {
        board: Arc::clone(board),
        calls: Mutex::new(Vec::new()),
        secrets: BTreeMap::new(),
        failing: BTreeSet::new(),
    }
}

fn recording_email(board: &Arc<Board>) -> RecordingEmail {
    RecordingEmail {
        board: Arc::clone(board),
        created: Mutex::new(Vec::new()),
        rotated: Mutex::new(Vec::new()),
        fail_created: AtomicBool::new(false),
        fail_rotated: AtomicBool::new(false),
    }
}

fn key_audit(enabled: bool, scheduler: &Arc<RecordingScheduler>) -> KeyEventAudit {
    let sink: Arc<dyn KeyAuditScheduler> = Arc::clone(scheduler);
    KeyEventAudit {
        settings: Arc::new(TestAuditSettings { enabled }),
        clock: Arc::new(FixedClock),
        new_id: Arc::new(|| "audit-1".to_owned()),
        scheduler: sink,
        litellm_proxy_admin_name: None,
    }
}

pub(super) struct SinkFixture {
    pub(super) audit: KeyEventAudit,
    pub(super) spawner: Arc<CollectedSpawner>,
    pub(super) repository: Arc<RecordingRepository>,
}

// the audit of a case runs B2330's whole sink, so what it stores is what the proxy would store
pub(super) fn sink_fixture() -> SinkFixture {
    let spawner = Arc::new(CollectedSpawner::default());
    let repository = Arc::new(RecordingRepository::default());
    let tasks: Arc<dyn AuditTaskSpawner> = Arc::clone(&spawner);
    let stored: Arc<dyn AuditLogRepository> = Arc::clone(&repository);
    let sink = AuditEventSink {
        settings: Arc::new(TestAuditSettings { enabled: true }),
        callbacks: Arc::new(AuditCallbackDispatcher::new(
            Arc::new(NoRegistry),
            Arc::clone(&tasks),
        )),
        repository: stored,
        clock: Arc::new(FixedClock),
        new_id: Arc::new(|| "audit-1".to_owned()),
    };
    let scheduler: Arc<dyn KeyAuditScheduler> = Arc::new(SinkScheduler {
        sink: Arc::new(sink),
        spawner: Arc::clone(&tasks),
    });
    let audit = KeyEventAudit {
        settings: Arc::new(TestAuditSettings { enabled: true }),
        clock: Arc::new(FixedClock),
        new_id: Arc::new(|| "audit-1".to_owned()),
        scheduler,
        litellm_proxy_admin_name: None,
    };
    SinkFixture {
        audit,
        spawner,
        repository,
    }
}

impl Harness {
    #[must_use]
    pub(super) fn with_teams(mut self) -> Self {
        self.teams = Some(ScriptedTeams {
            board: Arc::clone(&self.board),
            lookups: Mutex::new(Vec::new()),
        });
        self
    }

    pub(super) async fn dispatch(
        &self,
        event: &KeyEvent,
        caller: &KeyEventCaller,
    ) -> Result<(), KeyEventError> {
        self.dispatch_as(event, caller, None).await
    }

    pub(super) async fn dispatch_as(
        &self,
        event: &KeyEvent,
        caller: &KeyEventCaller,
        litellm_changed_by: Option<&str>,
    ) -> Result<(), KeyEventError> {
        KeyEvents {
            audit: &self.audit,
            effects: self.effects(),
        }
        .dispatch(event, caller, litellm_changed_by)
        .await
    }

    fn effects(&self) -> KeyEventEffects<'_, FakeManager> {
        let directory: Option<&dyn TeamDirectory> =
            self.teams.as_ref().map(|teams| teams as &dyn TeamDirectory);
        KeyEventEffects {
            secrets: SecretSink {
                settings: &self.settings,
                manager: self.client.then_some(&self.manager),
                teams: directory,
            },
            email: &self.email,
        }
    }

    pub(super) fn steps(&self) -> Vec<String> {
        self.board.steps()
    }

    pub(super) fn records(&self) -> Vec<AuditLogRecord> {
        lock(&self.scheduler.records).clone()
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        lock(&self.manager.calls).clone()
    }

    pub(super) fn warnings(&self) -> Vec<String> {
        self.logs
            .lines()
            .into_iter()
            .map(|(_, text)| text)
            .collect()
    }

    pub(super) fn created_emails(&self) -> usize {
        lock(&self.email.created).len()
    }

    pub(super) fn rotated_emails(&self) -> usize {
        lock(&self.email.rotated).len()
    }

    pub(super) fn team_lookups(&self) -> Vec<String> {
        self.teams
            .as_ref()
            .map(|teams| lock(&teams.lookups).clone())
            .unwrap_or_default()
    }
}

pub(super) fn caller() -> KeyEventCaller {
    KeyEventCaller {
        identity: CallerIdentity {
            user_id: Some("user-1".to_owned()),
            api_key: Some(SecretString::from("caller-api-key")),
            metadata: None,
            team_metadata: None,
        },
        token: Some(SecretString::from("caller-token")),
    }
}
