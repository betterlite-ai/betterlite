use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::audit_events::Settings;
use betterlite_models::decisions::Json;
use betterlite_server::audit_events::callbacks::{AuditCallbackFuture, AuditTaskSpawner};
use betterlite_server::audit_events::identity::CallerIdentity;
use betterlite_server::audit_events::record::AuditLogRecord;
use betterlite_server::audit_events::settings::ConfiguredAuditSettings;
use betterlite_server::audit_events::sink::{AuditClock, AuditRepositoryError};
use betterlite_server::team_callback_policy::audit::{
    TeamAuditWriter, TeamCallbackAudit, TeamCallbackChange,
};
use betterlite_server::team_callback_policy::error::TeamCallbackError;
use chrono::{DateTime, FixedOffset};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Runtime;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuditCase {
    store_audit_logs: bool,
    team_id: String,
    before: Json,
    after: Json,
    user_id: Option<String>,
    api_key: Option<String>,
    litellm_changed_by: Option<String>,
    admin_name: Option<String>,
    outcome: Outcome,
    now: DateTime<FixedOffset>,
}

// written stores the row, failed raises from the write, cancelled ends the task before the write finishes
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Written,
    Failed,
    Cancelled,
}

#[derive(Debug, Serialize)]
pub(super) struct AuditAnswer {
    scheduled: bool,
    record: Option<Row>,
}

#[derive(Debug, Serialize)]
struct Row {
    id: String,
    updated_at: String,
    changed_by: Option<String>,
    changed_by_api_key: Option<String>,
    action: &'static str,
    table_name: &'static str,
    object_id: String,
    before_value: Option<Value>,
    updated_values: Option<Value>,
}

struct CaseClock(DateTime<FixedOffset>);

impl AuditClock for CaseClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

#[derive(Default)]
struct HeldTasks(Mutex<Vec<AuditCallbackFuture>>);

impl AuditTaskSpawner for HeldTasks {
    fn spawn(&self, task: AuditCallbackFuture) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(task);
    }
}

// the create_audit_log_for_update the case injects in place of the database
struct CaseWriter {
    outcome: Outcome,
    rows: Mutex<Vec<AuditLogRecord>>,
}

#[async_trait]
impl TeamAuditWriter for CaseWriter {
    async fn write(&self, record: AuditLogRecord) -> Result<(), AuditRepositoryError> {
        match self.outcome {
            Outcome::Failed => Err(AuditRepositoryError::Unavailable),
            Outcome::Written | Outcome::Cancelled => {
                self.rows
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(record);
                Ok(())
            }
        }
    }
}

// the emit under test, with the case's clock, settings and writer in place of the proxy's
fn audit_log(
    case: &AuditCase,
    writer: Arc<CaseWriter>,
    tasks: Arc<HeldTasks>,
) -> TeamCallbackAudit {
    let config = Settings {
        store_audit_logs: Some(case.store_audit_logs),
        store_audit_logs_env: None,
        audit_log_callbacks: Vec::new(),
        s3_audit_callback_params: None,
    };
    TeamCallbackAudit {
        settings: Arc::new(ConfiguredAuditSettings {
            config,
            callbacks: Vec::new(),
        }),
        clock: Arc::new(CaseClock(case.now)),
        new_id: Arc::new(|| "betterlite-audit-row".to_owned()),
        writer,
        spawner: tasks,
        litellm_proxy_admin_name: case.admin_name.clone(),
    }
}

pub(super) fn audit(case: &AuditCase, runtime: &Runtime) -> Result<AuditAnswer, TeamCallbackError> {
    let tasks = Arc::new(HeldTasks::default());
    let writer = Arc::new(CaseWriter {
        outcome: case.outcome,
        rows: Mutex::new(Vec::new()),
    });
    let audit_log = audit_log(case, Arc::clone(&writer), Arc::clone(&tasks));
    let caller = CallerIdentity {
        user_id: case.user_id.clone(),
        api_key: case.api_key.as_deref().map(SecretString::from),
        metadata: None,
        team_metadata: None,
    };
    audit_log.emit(&TeamCallbackChange {
        team_id: &case.team_id,
        before_metadata: &case.before,
        after_metadata: &case.after,
        caller: &caller,
        litellm_changed_by: case.litellm_changed_by.as_deref(),
    })?;
    let held = mem::take(&mut *tasks.0.lock().unwrap_or_else(PoisonError::into_inner));
    let scheduled = !held.is_empty();
    match case.outcome {
        Outcome::Cancelled => drop(held),
        Outcome::Written | Outcome::Failed => runtime.block_on(async {
            for task in held {
                task.await;
            }
        }),
    }
    let rows = writer.rows.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(AuditAnswer {
        scheduled,
        record: rows.first().map(row),
    })
}

fn row(record: &AuditLogRecord) -> Row {
    Row {
        id: record.id.clone(),
        updated_at: record.updated_at.to_rfc3339(),
        changed_by: record.changed_by.clone(),
        changed_by_api_key: record
            .changed_by_api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        action: record.action.as_str(),
        table_name: record.table_name.as_str(),
        object_id: record.object_id.clone(),
        before_value: record.before_value.clone(),
        updated_values: record.updated_values.clone(),
    }
}
