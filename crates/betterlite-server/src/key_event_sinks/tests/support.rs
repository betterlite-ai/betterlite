use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use betterlite_auth::auth_team_lookup::ports::{StoreAnswer, TeamCache, TeamCacheError, TeamStore};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_config::sections::key_event_sinks::Settings;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_logging::budget_email::event::WebhookEvent;
use betterlite_secret_managers::secret_contracts::options::{
    DeleteOptions, SecretOptions, SecretRotation, SecretTags, SecretWrite,
};
use betterlite_secret_managers::secret_contracts::{
    BaseSecretManager, RotationError, SecretFuture,
};
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::Notify;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::key_event_sinks::ports::{KeyCreatedMailer, TeamDirectory};

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn settings(store_virtual_keys: bool, prefix: &str) -> Settings {
    Settings {
        store_virtual_keys,
        prefix_for_stored_virtual_keys: prefix.to_owned(),
        description: None,
        tags: None,
        alerting: Vec::new(),
    }
}

pub(super) fn team(team_id: &str, metadata: &str) -> LiteLLMTeamTableCachedObj {
    serde_json::from_str(&format!(
        r#"{{"team_id": "{team_id}", "metadata": {metadata}}}"#
    ))
    .unwrap()
}

pub(super) fn object(text: &str) -> BTreeMap<String, JsonData> {
    serde_json::from_str(text).unwrap()
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Call<P> {
    Read {
        name: String,
        params: Option<P>,
    },
    Write {
        name: String,
        value: String,
        description: Option<String>,
        tags: Option<SecretTags>,
        params: Option<P>,
    },
    Delete {
        name: String,
        window: u32,
        params: Option<P>,
    },
    Rotate {
        current: String,
        new: String,
        value: String,
        params: Option<P>,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("the secret manager refused {name}")]
pub(super) struct Refused {
    pub(super) name: String,
}

pub(super) struct FakeManager<P> {
    pub(super) calls: Mutex<Vec<Call<P>>>,
    pub(super) secrets: BTreeMap<String, String>,
    pub(super) failing: BTreeSet<String>,
}

pub(super) fn manager<P>(secrets: &[(&str, &str)], failing: &[&str]) -> FakeManager<P> {
    FakeManager {
        calls: Mutex::new(Vec::new()),
        secrets: secrets
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        failing: failing.iter().map(|name| (*name).to_owned()).collect(),
    }
}

impl<P: Clone> FakeManager<P> {
    pub(super) fn calls(&self) -> Vec<Call<P>> {
        lock(&self.calls).clone()
    }

    fn record(&self, call: Call<P>, name: &str) -> Result<(), Refused> {
        lock(&self.calls).push(call);
        if self.failing.contains(name) {
            return Err(Refused {
                name: name.to_owned(),
            });
        }
        Ok(())
    }
}

impl<P: Clone + Send + Sync> BaseSecretManager for FakeManager<P> {
    type Params = P;
    type Response = ();
    type Error = Refused;

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: SecretOptions<'borrowed, P>,
    ) -> SecretFuture<'borrowed, Option<SecretString>, Refused> {
        Box::pin(future::ready(self.sync_read_secret(secret_name, options)))
    }

    fn sync_read_secret(
        &self,
        secret_name: &str,
        options: SecretOptions<'_, P>,
    ) -> Result<Option<SecretString>, Refused> {
        let params = options.optional_params.cloned();
        self.record(
            Call::Read {
                name: secret_name.to_owned(),
                params,
            },
            secret_name,
        )?;
        Ok(self
            .secrets
            .get(secret_name)
            .map(|value| SecretString::from(value.as_str())))
    }

    fn async_write_secret(&self, write: SecretWrite<'_, P>) -> SecretFuture<'_, (), Refused> {
        let call = Call::Write {
            name: write.secret_name.to_owned(),
            value: write.secret_value.expose_secret().to_owned(),
            description: write.description.map(str::to_owned),
            tags: write.tags.cloned(),
            params: write.options.optional_params.cloned(),
        };
        Box::pin(future::ready(self.record(call, write.secret_name)))
    }

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: DeleteOptions<'borrowed, P>,
    ) -> SecretFuture<'borrowed, (), Refused> {
        let params = options.operation.optional_params.cloned();
        let call = Call::Delete {
            name: secret_name.to_owned(),
            window: options.recovery_window_in_days,
            params,
        };
        Box::pin(future::ready(self.record(call, secret_name)))
    }

    fn async_rotate_secret(
        &self,
        rotation: SecretRotation<'_, P>,
    ) -> SecretFuture<'_, (), RotationError<Refused>> {
        let call = Call::Rotate {
            current: rotation.current_secret_name.to_owned(),
            new: rotation.new_secret_name.to_owned(),
            value: rotation.new_secret_value.expose_secret().to_owned(),
            params: rotation.options.optional_params.cloned(),
        };
        let recorded = self.record(call, rotation.current_secret_name);
        Box::pin(future::ready(
            recorded.map_err(|source| RotationError::Operation { source }),
        ))
    }
}

#[derive(Default)]
pub(super) struct FakeTeams {
    pub(super) teams: BTreeMap<String, LiteLLMTeamTableCachedObj>,
    pub(super) lookups: Mutex<Vec<String>>,
}

#[async_trait]
impl TeamDirectory for FakeTeams {
    async fn team(
        &self,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamLookupError> {
        lock(&self.lookups).push(team_id.to_owned());
        let found = self.teams.get(team_id).cloned();
        found
            .map(Some)
            .ok_or_else(|| TeamLookupError::TeamNotFound {
                team_id: team_id.to_owned(),
            })
    }
}

// one fake behind every port of B737's lookup, so the production TeamDirectory runs over it
#[derive(Default)]
pub(super) struct CachedTeams {
    pub(super) team: Option<LiteLLMTeamTableCachedObj>,
    pub(super) reads: Mutex<Vec<String>>,
}

pub(super) fn team_lookup(fake: &Arc<CachedTeams>, database: bool) -> TeamLookup {
    TeamLookup {
        cache: Arc::<CachedTeams>::clone(fake),
        permission_cache: Arc::<CachedTeams>::clone(fake),
        database: database.then(|| TeamDatabase {
            store: Arc::<CachedTeams>::clone(fake),
            permission_reader: Arc::<CachedTeams>::clone(fake),
            permission_writer: Arc::<CachedTeams>::clone(fake),
        }),
        provisioner: None,
        cadence: Arc::new(Cadence::new(16, Arc::new(WallClock))),
        in_memory_ttl: 60,
    }
}

#[async_trait]
impl TeamCache for CachedTeams {
    async fn read_team(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        lock(&self.reads).push(key.to_owned());
        Ok(self.team.clone())
    }

    async fn write_team(
        &self,
        _key: &str,
        _team: &LiteLLMTeamTableCachedObj,
        _ttl: f64,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn cache_team(
        &self,
        _team_id: &str,
        _team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn read_access_group(
        &self,
        _key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        Ok(None)
    }

    async fn cache_access_group(
        &self,
        _access_group_id: &str,
        _table: &AccessGroupTable,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }
}

#[async_trait]
impl ObjectPermissionCache for CachedTeams {
    async fn read(&self, _key: &str) -> PermissionAnswer {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _permission: &LiteLLMObjectPermissionTable,
        _ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

#[async_trait]
impl TeamStore for CachedTeams {
    async fn find_team(&self, _team_id: &str, _use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        Ok(None)
    }

    async fn find_by_alias(&self, _team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        Ok(Vec::new())
    }

    async fn find_access_group(
        &self,
        _access_group_id: &str,
        _use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>> {
        Ok(None)
    }
}

#[async_trait]
impl ObjectPermissionStore for CachedTeams {
    async fn find_permission(&self, _id: &str) -> Result<Option<JsonData>, PermissionError> {
        Ok(None)
    }
}

// holds every email until the test opens the gate, so a send that waited would never return
#[derive(Default)]
pub(super) struct GatedMailer {
    pub(super) gate: Notify,
    pub(super) held: bool,
    pub(super) events: Mutex<Vec<WebhookEvent>>,
}

#[async_trait]
impl KeyCreatedMailer for GatedMailer {
    async fn send(&self, event: WebhookEvent) {
        if self.held {
            self.gate.notified().await;
        }
        lock(&self.events).push(event);
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs {
    pub(super) lines: Arc<Mutex<Vec<(Level, String)>>>,
}

impl Logs {
    pub(super) fn lines(&self) -> Vec<(Level, String)> {
        lock(&self.lines).clone()
    }
}

impl<S: Subscriber> Layer<S> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if !event
            .metadata()
            .target()
            .starts_with("betterlite_server::key_event_sinks")
        {
            return;
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        lock(&self.lines).push((*event.metadata().level(), message.0));
    }
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}
