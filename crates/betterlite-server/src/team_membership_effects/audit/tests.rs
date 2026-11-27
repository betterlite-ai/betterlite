#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::audit::Settings as AuditSettings;
use betterlite_config::sections::audit_events::Settings as SectionSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::team_rows::member::{Member, MemberBase, MemberRole};
use betterlite_logging::audit::callbacks::{
    AuditLoggerBuildError, DedicatedAuditLoggerBuilder, NamedAuditCallbacks,
};
use betterlite_logging::audit::delivery::AuditDelivery;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::registry::catalog::CatalogError;
use betterlite_logging::registry::error::RegistryError;
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use serde_json::json;

use super::{TeamMembershipAudit, members_audit_value, schedule_team_membership_audit_log};
use crate::audit::service::{AuditService, AuditServiceDeps};
use crate::audit_events::callbacks::{AuditCallbackFuture, AuditTaskSpawner};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::sink::{AuditClock, AuditLogRepository, AuditRepositoryError};

const NOW: &str = "2024-05-06T07:08:09.123+00:00";

struct FixedClock;

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(NOW).expect("the fixed clock is a timestamp")
    }
}

#[derive(Default)]
struct RecordingRepository {
    inserted: Mutex<Vec<AuditLogRecord>>,
}

impl RecordingRepository {
    fn inserted(&self) -> Vec<AuditLogRecord> {
        self.inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
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

struct NoNamedCallbacks;

impl LoggerResolver for NoNamedCallbacks {
    fn resolve(
        &self,
        _name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        Ok(None)
    }
}

impl DedicatedAuditLoggerBuilder for NoNamedCallbacks {
    fn build(&self, _params: &JsonValue) -> Result<Arc<dyn CallbackLogger>, AuditLoggerBuildError> {
        Err(AuditLoggerBuildError::Build {
            source: CatalogError::Init {
                message: "no dedicated audit logger in this test".to_owned(),
            },
        })
    }
}

// LiteLLM's asyncio.create_task is this spawner: it keeps the future and runs it only when the test says so
#[derive(Default)]
struct KeptSpawner {
    tasks: Mutex<Vec<AuditCallbackFuture>>,
}

impl KeptSpawner {
    fn kept(&self) -> usize {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    async fn run_all(&self) {
        let tasks: Vec<AuditCallbackFuture> = self
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect();
        for task in tasks {
            task.await;
        }
    }
}

impl AuditTaskSpawner for KeptSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(task);
    }
}

struct Harness {
    service: Arc<AuditService>,
    repository: Arc<RecordingRepository>,
    spawner: Arc<KeptSpawner>,
}

impl Harness {
    fn schedule(&self, entry: &TeamMembershipAudit<'_>) -> Result<(), PrivateJsonError> {
        let spawner: Arc<dyn AuditTaskSpawner> = Arc::<KeptSpawner>::clone(&self.spawner);
        schedule_team_membership_audit_log(&self.service, &spawner, entry)
    }
}

fn worker_settings() -> WorkerSettings {
    WorkerSettings {
        max_queue_size: 8,
        concurrency: 1,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(5),
        timeout_summary_window: Duration::from_secs(1),
    }
}

fn settings(store_audit_logs: bool) -> AuditSettings {
    AuditSettings {
        config: SectionSettings {
            store_audit_logs: Some(store_audit_logs),
            store_audit_logs_env: None,
            audit_log_callbacks: Vec::new(),
            s3_audit_callback_params: None,
        },
        audit_log_callbacks_present: false,
    }
}

fn harness(store_audit_logs: bool) -> Harness {
    let repository = Arc::new(RecordingRepository::default());
    let delivery = Arc::new(AuditDelivery {
        named: NamedAuditCallbacks {
            resolver: Arc::new(NoNamedCallbacks),
            dedicated: Arc::new(NoNamedCallbacks),
            cache: Mutex::new(BTreeMap::new()),
        },
        worker: LoggingWorker::start(worker_settings()).expect("the delivery worker starts"),
    });
    let service = Arc::new(
        AuditService::start(AuditServiceDeps {
            settings: settings(store_audit_logs),
            callbacks: Vec::new(),
            delivery,
            repository: Arc::<RecordingRepository>::clone(&repository),
            clock: Arc::new(FixedClock),
            new_id: Arc::new(|| "test-audit-id".to_owned()),
            worker_settings: worker_settings(),
        })
        .expect("the audit service starts"),
    );
    Harness {
        service,
        repository,
        spawner: Arc::new(KeptSpawner::default()),
    }
}

fn member(user_id: &str, email: &str, role: MemberRole) -> Member {
    Member {
        base: MemberBase {
            user_id: Some(user_id.to_owned()),
            user_email: Some(email.to_owned()),
        },
        role,
    }
}

fn before_roster() -> Vec<Member> {
    vec![
        member("test-user-1", "test-1@example.com", MemberRole::Admin),
        member("test-user-2", "test-2@example.com", MemberRole::User),
    ]
}

fn after_roster() -> Vec<Member> {
    vec![member(
        "test-user-1",
        "test-1@example.com",
        MemberRole::Admin,
    )]
}

fn caller(user_id: Option<&str>, metadata: Option<JsonValue>) -> CallerIdentity {
    CallerIdentity {
        user_id: user_id.map(str::to_owned),
        api_key: Some(SecretString::from("sk-test-key".to_owned())),
        metadata,
        team_metadata: None,
    }
}

fn entry<'entry>(
    caller: &'entry CallerIdentity,
    before: &'entry [Member],
    after: &'entry [Member],
) -> TeamMembershipAudit<'entry> {
    TeamMembershipAudit {
        team_id: "test-team-1",
        team_alias: Some("Test Team"),
        before_members: before,
        after_members: after,
        caller,
        litellm_proxy_admin_name: Some("test-proxy-admin"),
    }
}

fn parsed(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("the audit value is JSON")
}

#[test]
fn members_audit_value_nests_the_roster_beside_the_alias() {
    let text = members_audit_value(Some("Test Team"), &before_roster()).unwrap();
    assert_eq!(
        parsed(&text),
        json!({
            "team_alias": "Test Team",
            "members_with_roles": [
                {
                    "user_id": "test-user-1",
                    "user_email": "test-1@example.com",
                    "role": "admin",
                },
                {
                    "user_id": "test-user-2",
                    "user_email": "test-2@example.com",
                    "role": "user",
                },
            ],
        })
    );
}

#[test]
fn members_audit_value_keeps_a_null_alias_and_null_member_fields() {
    let members = vec![
        Member {
            base: MemberBase {
                user_id: None,
                user_email: Some("test-email-only@example.com".to_owned()),
            },
            role: MemberRole::User,
        },
        Member {
            base: MemberBase {
                user_id: Some("test-user-only".to_owned()),
                user_email: None,
            },
            role: MemberRole::Admin,
        },
    ];
    let text = members_audit_value(None, &members).unwrap();
    assert_eq!(
        parsed(&text),
        json!({
            "team_alias": null,
            "members_with_roles": [
                {
                    "user_id": null,
                    "user_email": "test-email-only@example.com",
                    "role": "user",
                },
                {
                    "user_id": "test-user-only",
                    "user_email": null,
                    "role": "admin",
                },
            ],
        })
    );
}

#[test]
fn members_audit_value_keeps_the_roster_order() {
    let members = vec![
        member("test-user-3", "test-3@example.com", MemberRole::User),
        member("test-user-1", "test-1@example.com", MemberRole::Admin),
        member("test-user-2", "test-2@example.com", MemberRole::User),
    ];
    let text = members_audit_value(Some("Test Team"), &members).unwrap();
    let value = parsed(&text);
    let listed: Vec<&str> = value["members_with_roles"]
        .as_array()
        .expect("members_with_roles is an array")
        .iter()
        .filter_map(|member| member["user_id"].as_str())
        .collect();
    assert_eq!(listed, ["test-user-3", "test-user-1", "test-user-2"]);
}

#[test]
fn members_audit_value_drops_nul_bytes_from_member_text() {
    let members = vec![member(
        "test-user-\0one",
        "test-1@example.com",
        MemberRole::User,
    )];
    let text = members_audit_value(Some("Test Team"), &members).unwrap();
    let value = parsed(&text);
    assert_eq!(value["members_with_roles"][0]["user_id"], "test-user-one");
}

#[tokio::test]
async fn disabled_audit_logging_spawns_nothing() {
    let harness = harness(false);
    let caller = caller(Some("test-admin"), None);
    let before = before_roster();
    let after = after_roster();
    harness.schedule(&entry(&caller, &before, &after)).unwrap();
    assert_eq!(harness.spawner.kept(), 0);
    harness.spawner.run_all().await;
    assert!(harness.repository.inserted().is_empty());
}

#[tokio::test]
async fn an_unchanged_roster_spawns_nothing() {
    let harness = harness(true);
    let caller = caller(Some("test-admin"), None);
    let before = before_roster();
    let after = before.clone();
    harness.schedule(&entry(&caller, &before, &after)).unwrap();
    assert_eq!(harness.spawner.kept(), 0);
}

#[tokio::test]
async fn a_reordered_roster_is_a_change() {
    let harness = harness(true);
    let caller = caller(Some("test-admin"), None);
    let before = before_roster();
    let mut after = before.clone();
    after.reverse();
    harness.schedule(&entry(&caller, &before, &after)).unwrap();
    assert_eq!(harness.spawner.kept(), 1);
}

#[tokio::test]
async fn a_changed_roster_schedules_one_updated_team_row() {
    let harness = harness(true);
    let caller = caller(Some("test-admin"), None);
    let before = before_roster();
    let after = after_roster();
    harness.schedule(&entry(&caller, &before, &after)).unwrap();
    assert_eq!(harness.spawner.kept(), 1);
    assert!(harness.repository.inserted().is_empty());
    harness.spawner.run_all().await;
    let inserted = harness.repository.inserted();
    assert_eq!(inserted.len(), 1);
    let record = inserted.first().unwrap();
    assert_eq!(record.action, AuditAction::Updated);
    assert_eq!(record.table_name, AuditTable::Team);
    assert_eq!(record.object_id, "test-team-1");
    assert_eq!(record.changed_by.as_deref(), Some("test-admin"));
    assert_eq!(
        record.before_value,
        Some(json!({
            "team_alias": "Test Team",
            "members_with_roles": [
                {
                    "user_id": "test-user-1",
                    "user_email": "test-1@example.com",
                    "role": "admin",
                },
                {
                    "user_id": "test-user-2",
                    "user_email": "test-2@example.com",
                    "role": "user",
                },
            ],
        }))
    );
    assert_eq!(
        record.updated_values,
        Some(json!({
            "team_alias": "Test Team",
            "members_with_roles": [
                {
                    "user_id": "test-user-1",
                    "user_email": "test-1@example.com",
                    "role": "admin",
                },
            ],
        }))
    );
}

#[tokio::test]
async fn the_membership_row_ignores_the_changed_by_header() {
    let harness = harness(true);
    let caller = caller(
        Some("test-admin"),
        Some(json!({"allow_litellm_changed_by_header": true})),
    );
    let before = before_roster();
    let after = after_roster();
    harness.schedule(&entry(&caller, &before, &after)).unwrap();
    harness.spawner.run_all().await;
    let inserted = harness.repository.inserted();
    let record = inserted.first().unwrap();
    assert_eq!(record.changed_by.as_deref(), Some("test-admin"));
}

#[tokio::test]
async fn a_missing_caller_falls_back_to_the_proxy_admin_name() {
    let harness = harness(true);
    let caller = caller(None, None);
    let before = before_roster();
    let after = after_roster();
    harness.schedule(&entry(&caller, &before, &after)).unwrap();
    harness.spawner.run_all().await;
    let inserted = harness.repository.inserted();
    let record = inserted.first().unwrap();
    assert_eq!(record.changed_by.as_deref(), Some("test-proxy-admin"));
}
