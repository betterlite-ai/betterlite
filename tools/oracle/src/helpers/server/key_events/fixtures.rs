use std::collections::{BTreeMap, BTreeSet};
use std::future;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_logging::budget_email::event::WebhookEvent;
use betterlite_secret_managers::secret_contracts::options::{
    DeleteOptions, SecretOptions, SecretRotation, SecretWrite,
};
use betterlite_secret_managers::secret_contracts::{
    BaseSecretManager, RotationError, SecretFuture,
};
use betterlite_server::audit_events::record::{AuditLogRecord, timestamp_text};
use betterlite_server::key_event_sinks::email::{CreatedKey, LegacyEmail};
use betterlite_server::key_event_sinks::ports::{KeyCreatedMailer, TeamDirectory};
use betterlite_server::key_event_sinks::team_params::TeamSecretSettings;
use betterlite_server::key_events::audit::KeyAuditScheduler;
use betterlite_server::key_events::effects::{KeyEmailError, KeyEventEmail};
use rust_decimal::prelude::ToPrimitive;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tokio_util::task::TaskTracker;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// one board holds what every collaborator saw, in the order the hook reached it
#[derive(Default)]
pub(super) struct Board {
    trace: Mutex<Vec<Value>>,
}

impl Board {
    fn record(&self, entry: Value) {
        self.trace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(super) fn entries(&self) -> Vec<Value> {
        self.trace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted secret manager refused {name}")]
pub(super) struct Refused {
    name: String,
}

pub(super) struct ScriptedManager {
    pub(super) board: Arc<Board>,
    pub(super) secrets: BTreeMap<String, String>,
    pub(super) failing: BTreeSet<String>,
}

impl ScriptedManager {
    fn answer(&self, effect: Value, name: &str) -> Result<(), Refused> {
        self.board.record(effect);
        if self.failing.contains(name) {
            return Err(Refused {
                name: name.to_owned(),
            });
        }
        Ok(())
    }
}

impl BaseSecretManager for ScriptedManager {
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
        options: SecretOptions<'_, TeamSecretSettings>,
    ) -> Result<Option<SecretString>, Refused> {
        self.answer(
            json!(["read", secret_name, options.optional_params]),
            secret_name,
        )?;
        Ok(self
            .secrets
            .get(secret_name)
            .map(|value| SecretString::from(value.as_str())))
    }

    fn async_write_secret(
        &self,
        write: SecretWrite<'_, TeamSecretSettings>,
    ) -> SecretFuture<'_, (), Refused> {
        let value = write.secret_value.expose_secret();
        let effect = json!([
            "write",
            write.secret_name,
            value,
            write.description,
            write.tags,
            write.options.optional_params
        ]);
        Box::pin(future::ready(self.answer(effect, write.secret_name)))
    }

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: DeleteOptions<'borrowed, TeamSecretSettings>,
    ) -> SecretFuture<'borrowed, (), Refused> {
        let effect = json!([
            "delete",
            secret_name,
            options.recovery_window_in_days,
            options.operation.optional_params
        ]);
        Box::pin(future::ready(self.answer(effect, secret_name)))
    }

    fn async_rotate_secret(
        &self,
        rotation: SecretRotation<'_, TeamSecretSettings>,
    ) -> SecretFuture<'_, (), RotationError<Refused>> {
        let value = rotation.new_secret_value.expose_secret();
        let effect = json!([
            "rotate",
            rotation.current_secret_name,
            rotation.new_secret_name,
            value,
            rotation.options.optional_params
        ]);
        let answer = self.answer(effect, rotation.current_secret_name);
        Box::pin(future::ready(
            answer.map_err(|source| RotationError::Operation { source }),
        ))
    }
}

pub(super) struct ScriptedTeams {
    pub(super) board: Arc<Board>,
    pub(super) metadata: BTreeMap<String, JsonData>,
    pub(super) database: bool,
}

#[async_trait]
impl TeamDirectory for ScriptedTeams {
    async fn team(
        &self,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamLookupError> {
        if !self.database {
            return Ok(None);
        }
        self.board.record(json!(["lookup", team_id]));
        let missing = || TeamLookupError::TeamNotFound {
            team_id: team_id.to_owned(),
        };
        let metadata = self.metadata.get(team_id).ok_or_else(missing)?;
        let row = BTreeMap::from([
            ("team_id".to_owned(), JsonData::String(team_id.to_owned())),
            ("metadata".to_owned(), metadata.clone()),
        ]);
        let team = LiteLLMTeamTableCachedObj::try_from(JsonData::Object(row))
            .ok()
            .ok_or_else(missing)?;
        Ok(Some(team))
    }
}

pub(super) struct BoardMailer {
    pub(super) board: Arc<Board>,
}

#[async_trait]
impl KeyCreatedMailer for BoardMailer {
    async fn send(&self, event: WebhookEvent) {
        let view = json!({
            "event": event.event.to_string(),
            "event_group": event.event_group.to_string(),
            "event_message": event.event_message,
            "token": event.token.as_ref().map(ExposeSecret::expose_secret),
            "spend": event.spend.to_f64(),
            "max_budget": event.max_budget.as_ref().and_then(ToPrimitive::to_f64),
            "user_id": event.user_id,
            "team_id": event.team_id,
            "key_alias": event.key_alias,
        });
        self.board.record(json!(["email", view]));
    }
}

// B898's created-email decision is kept, and its task is awaited where the hook sends it, so the
// mail it records sits at the email step of the trace rather than after the hook returned
pub(super) struct WaitingEmail {
    pub(super) alerting: Vec<String>,
    pub(super) board: Arc<Board>,
    pub(super) tasks: TaskTracker,
}

#[async_trait]
impl KeyEventEmail for WaitingEmail {
    async fn send_created(&self, response: CreatedKey) -> Result<(), KeyEmailError> {
        let email = LegacyEmail {
            alerting: &self.alerting,
            mailer: Arc::new(BoardMailer {
                board: Arc::clone(&self.board),
            }),
            tasks: &self.tasks,
        };
        let sent = email.send_key_created_email(response);
        self.tasks.close();
        self.tasks.wait().await;
        sent.map_err(KeyEmailError::from)
    }

    async fn send_rotated(
        &self,
        _response: &str,
        _existing_key_alias: Option<&str>,
    ) -> Result<(), KeyEmailError> {
        Ok(())
    }
}

pub(super) struct RecordingScheduler {
    pub(super) board: Arc<Board>,
}

impl KeyAuditScheduler for RecordingScheduler {
    fn schedule(&self, record: AuditLogRecord) {
        self.board.record(json!(["audit", record_view(&record)]));
    }
}

fn record_view(record: &AuditLogRecord) -> Value {
    json!({
        "id": record.id,
        "updated_at": timestamp_text(record.updated_at, "Z"),
        "changed_by": record.changed_by,
        "changed_by_api_key": record.changed_by_api_key.as_ref().map(ExposeSecret::expose_secret),
        "action": record.action.as_str(),
        "table_name": record.table_name.as_str(),
        "object_id": record.object_id,
        "before_value": record.before_value,
        "updated_values": record.updated_values,
    })
}

pub(super) struct Levels {
    pub(super) board: Arc<Board>,
}

impl<S: Subscriber> Layer<S> for Levels {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        if *metadata.level() != Level::WARN
            || !metadata
                .target()
                .starts_with("betterlite_server::key_event")
        {
            return;
        }
        // LiteLLM's warning text names its private helper and its arguments, so the trace orders
        // warnings by level alone, as the B898 trace does
        self.board.record(json!(["warn"]));
    }
}
