use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::DbError;
use indexmap::IndexMap;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry;

use super::super::repository::{
    ContainerObjectTable, ContainerOwnerRow, ContainerOwnerUpdate, ContainerOwnerWrite,
};
use super::super::response::synthetic_container;
use super::super::{CONTAINER_OBJECT_PURPOSE, ContainerListPage, ContainerListSource};

pub(crate) fn principal(user: &str) -> Principal {
    let document = format!(
        r#"{{"principal_type":"human","subject":"{user}","auth_method":"api_key","user":{{"id":"{user}"}},"teams":[{{"id":"team-1"}}],"organization":{{"id":"org-1"}},"credential_ref":{{"key_id":"key-1","token_id":"token-1"}}}}"#
    );
    serde_json::from_str(&document).unwrap()
}

pub(crate) fn admin(user: &str) -> Principal {
    let mut identity = principal(user);
    identity.roles = vec![Role::PlatformAdmin];
    identity
}

pub(crate) fn anonymous() -> Principal {
    serde_json::from_str(r#"{"principal_type":"human","subject":"anon","auth_method":"api_key"}"#)
        .unwrap()
}

pub(crate) fn anonymous_admin() -> Principal {
    let mut identity = anonymous();
    identity.roles = vec![Role::PlatformAdmin];
    identity
}

pub(crate) fn fixed_clock() -> Clock {
    Arc::new(|| 1_000.0_f64)
}

pub(crate) fn movable_clock() -> (Clock, Arc<Mutex<f64>>) {
    let value = Arc::new(Mutex::new(1_000.0_f64));
    let reading = Arc::clone(&value);
    let clock: Clock = Arc::new(move || *reading.lock().unwrap_or_else(PoisonError::into_inner));
    (clock, value)
}

pub(crate) fn advance(value: &Arc<Mutex<f64>>, now: f64) {
    *value.lock().unwrap_or_else(PoisonError::into_inner) = now;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    FindUnique(String),
    FindContainer(String),
    FindOwned(Vec<String>),
    Create(Box<ContainerOwnerWrite>),
    Update(Box<ContainerOwnerUpdate>),
}

#[derive(Debug)]
pub(crate) struct State {
    pub(crate) available: bool,
    pub(crate) effects: Vec<Effect>,
    pub(crate) rows: BTreeMap<String, ContainerOwnerRow>,
    pub(crate) owned: Vec<String>,
    pub(crate) failure: Option<&'static str>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            available: true,
            effects: Vec::new(),
            rows: BTreeMap::new(),
            owned: Vec::new(),
            failure: None,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ScriptedObjects {
    pub(crate) state: Arc<Mutex<State>>,
}

impl ScriptedObjects {
    pub(crate) fn unavailable() -> Self {
        let objects = Self::default();
        objects.update(|state| state.available = false);
        objects
    }

    pub(crate) fn update<Change>(&self, change: Change)
    where
        Change: FnOnce(&mut State),
    {
        change(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner));
    }

    pub(crate) fn effects(&self) -> Vec<Effect> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .clone()
    }

    pub(crate) fn row_for(
        &self,
        model_object_id: &str,
        file_purpose: &str,
        created_by: Option<&str>,
    ) {
        self.update(|state| {
            state.rows.insert(
                model_object_id.to_owned(),
                ContainerOwnerRow {
                    unified_object_id: Some("cntr_stored".to_owned()),
                    model_object_id: model_object_id.to_owned(),
                    file_purpose: file_purpose.to_owned(),
                    created_by: created_by.map(str::to_owned),
                },
            );
        });
    }

    pub(crate) fn set_stored_id(&self, model_object_id: &str, unified_object_id: Option<&str>) {
        self.update(|state| {
            if let Some(row) = state.rows.get_mut(model_object_id) {
                row.unified_object_id = unified_object_id.map(str::to_owned);
            }
        });
    }

    pub(crate) fn owned_scopes(&self, model_object_ids: &[String]) {
        self.update(|state| state.owned = model_object_ids.to_vec());
    }

    pub(crate) fn created(&self) -> Vec<ContainerOwnerWrite> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Create(write) => Some(*write),
                Effect::FindUnique(_)
                | Effect::FindContainer(_)
                | Effect::FindOwned(_)
                | Effect::Update(_) => None,
            })
            .collect()
    }

    pub(crate) fn updates(&self) -> Vec<ContainerOwnerUpdate> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Update(update) => Some(*update),
                Effect::FindUnique(_)
                | Effect::FindContainer(_)
                | Effect::FindOwned(_)
                | Effect::Create(_) => None,
            })
            .collect()
    }

    fn found_unique(&self, model_object_id: &str) -> Result<Option<ContainerOwnerRow>, TableError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .effects
            .push(Effect::FindUnique(model_object_id.to_owned()));
        let found = state.rows.get(model_object_id).cloned();
        let failure = state.failure;
        drop(state);
        outcome(failure, found)
    }

    fn found_container(
        &self,
        model_object_id: &str,
    ) -> Result<Option<ContainerOwnerRow>, TableError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .effects
            .push(Effect::FindContainer(model_object_id.to_owned()));
        let found = state
            .rows
            .get(model_object_id)
            .filter(|row| row.file_purpose == CONTAINER_OBJECT_PURPOSE)
            .cloned();
        let failure = state.failure;
        drop(state);
        outcome(failure, found)
    }

    fn found_owned(&self, owner_scopes: &[String]) -> Result<Vec<String>, TableError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push(Effect::FindOwned(owner_scopes.to_vec()));
        let owned = state.owned.clone();
        let failure = state.failure;
        drop(state);
        outcome(failure, owned)
    }

    fn written(&self, write: &ContainerOwnerWrite) -> Result<(), TableError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push(Effect::Create(Box::new(write.clone())));
        let failure = state.failure;
        if failure.is_none() {
            state
                .rows
                .insert(write.model_object_id.clone(), row_of(write));
            state.owned.push(write.model_object_id.clone());
        }
        drop(state);
        outcome(failure, ())
    }

    fn changed(&self, update: &ContainerOwnerUpdate) -> Result<(), TableError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push(Effect::Update(Box::new(update.clone())));
        let failure = state.failure;
        let mut missing = false;
        if failure.is_none() {
            match state.rows.get_mut(&update.model_object_id) {
                Some(row) => row.unified_object_id = Some(update.unified_object_id.clone()),
                None => missing = true,
            }
        }
        drop(state);
        if missing {
            return Err(TableError::MissingRow);
        }
        outcome(failure, ())
    }
}

impl ContainerObjectTable for ScriptedObjects {
    fn availability(&self) -> Result<(), TableError> {
        let available = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .available;
        if available {
            return Ok(());
        }
        Err(TableError::Database(DbError::NoDbConnected))
    }

    fn find_unique(
        &self,
        model_object_id: &str,
    ) -> impl Future<Output = Result<Option<ContainerOwnerRow>, TableError>> + Send {
        ready(self.found_unique(model_object_id))
    }

    fn find_container(
        &self,
        model_object_id: &str,
    ) -> impl Future<Output = Result<Option<ContainerOwnerRow>, TableError>> + Send {
        ready(self.found_container(model_object_id))
    }

    fn find_owned(
        &self,
        owner_scopes: Vec<String>,
    ) -> impl Future<Output = Result<Vec<String>, TableError>> + Send {
        ready(self.found_owned(&owner_scopes))
    }

    fn create(
        &self,
        write: ContainerOwnerWrite,
    ) -> impl Future<Output = Result<(), TableError>> + Send {
        ready(self.written(&write))
    }

    fn update(
        &self,
        update: ContainerOwnerUpdate,
    ) -> impl Future<Output = Result<(), TableError>> + Send {
        ready(self.changed(&update))
    }
}

fn outcome<Value>(failure: Option<&'static str>, value: Value) -> Result<Value, TableError> {
    match failure {
        Some(message) => Err(TableError::Publish {
            message: message.to_owned(),
        }),
        None => Ok(value),
    }
}

fn row_of(write: &ContainerOwnerWrite) -> ContainerOwnerRow {
    ContainerOwnerRow {
        unified_object_id: Some(write.unified_object_id.clone()),
        model_object_id: write.model_object_id.clone(),
        file_purpose: write.file_purpose.clone(),
        created_by: Some(write.created_by.clone()),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the upstream container list failed")]
pub(crate) struct PageFailure;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Call {
    pub(crate) after: Option<String>,
    pub(crate) limit: i64,
}

pub(crate) struct ScriptedPages {
    pages: Mutex<VecDeque<ContainerListPage>>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl ScriptedPages {
    pub(crate) fn new(pages: Vec<ContainerListPage>) -> Self {
        Self {
            pages: Mutex::new(VecDeque::from(pages)),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ContainerListSource for ScriptedPages {
    type Failure = PageFailure;

    fn fetch_page(
        &self,
        after: Option<&str>,
        limit: i64,
    ) -> impl Future<Output = Result<ContainerListPage, PageFailure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call {
                after: after.map(str::to_owned),
                limit,
            });
        let page = self
            .pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        ready(page.ok_or(PageFailure))
    }
}

pub(crate) fn page(ids: &[&str], has_more: bool) -> ContainerListPage {
    ContainerListPage {
        data: ids.iter().copied().map(synthetic_container).collect(),
        first_id: None,
        last_id: None,
        has_more,
        extra: IndexMap::new(),
    }
}

pub(crate) fn numbered_page(
    prefix: &str,
    start: usize,
    count: usize,
    has_more: bool,
) -> ContainerListPage {
    let end = start.saturating_add(count);
    let data = (start..end)
        .map(|index| synthetic_container(&format!("{prefix}{index}")))
        .collect();
    ContainerListPage {
        data,
        first_id: None,
        last_id: None,
        has_more,
        extra: IndexMap::new(),
    }
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<(Level, String)>>>);

impl Logs {
    pub(crate) fn messages(&self) -> Vec<(Level, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn warnings(&self) -> Vec<String> {
        self.at(Level::WARN)
    }

    pub(crate) fn errors(&self) -> Vec<String> {
        self.at(Level::ERROR)
    }

    fn at(&self, level: Level) -> Vec<String> {
        self.messages()
            .into_iter()
            .filter(|(seen, _)| *seen == level)
            .map(|(_, message)| message)
            .collect()
    }
}

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: tracing::Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        let mut field = MessageField::default();
        event.record(&mut field);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, field.0));
    }
}

pub(crate) fn capture_logs() -> (Logs, tracing::subscriber::DefaultGuard) {
    let logs = Logs::default();
    let guard = set_default(registry().with(logs.clone()));
    (logs, guard)
}
