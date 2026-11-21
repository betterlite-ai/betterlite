use std::collections::{BTreeMap, BTreeSet};
use std::future;
use std::mem;
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
use betterlite_server::key_event_sinks::ports::{KeyCreatedMailer, TeamDirectory};
use betterlite_server::key_event_sinks::team_params::TeamSecretSettings;
use rust_decimal::prelude::ToPrimitive;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// every collaborator writes to one board, so a step reads the whole call order
#[derive(Default)]
pub(super) struct Board {
    effects: Mutex<Vec<Value>>,
    logs: Mutex<Vec<String>>,
}

impl Board {
    fn record(&self, effect: Value) {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }

    // a step's answer holds what the board saw since the step before it
    pub(super) fn close_step(&self, raised: bool) -> Value {
        let effects = mem::take(&mut *self.effects.lock().unwrap_or_else(PoisonError::into_inner));
        let logs = mem::take(&mut *self.logs.lock().unwrap_or_else(PoisonError::into_inner));
        json!({"raised": raised, "effects": effects, "logs": logs})
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

    // one rotate effect, as the Python fake records it, keeps B4410's rotation steps out of these cases
    fn async_rotate_secret(
        &self,
        rotation: SecretRotation<'_, TeamSecretSettings>,
    ) -> SecretFuture<'_, (), RotationError<Refused>> {
        let value = rotation.new_secret_value.expose_secret();
        let params = rotation.options.optional_params;
        let effect = json!([
            "rotate",
            rotation.current_secret_name,
            rotation.new_secret_name,
            value,
            params
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

pub(super) struct Levels {
    pub(super) board: Arc<Board>,
}

impl<S: Subscriber> Layer<S> for Levels {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        if !metadata
            .target()
            .starts_with("betterlite_server::key_event_sinks")
        {
            return;
        }
        // Python's logging names the level WARNING where tracing writes WARN
        let level = if *metadata.level() == Level::WARN {
            "WARNING".to_owned()
        } else {
            metadata.level().to_string()
        };
        self.board
            .logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(level);
    }
}
