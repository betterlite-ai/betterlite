use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_auth::auth_team_lookup::ports::{
    StoreAnswer, TeamCache, TeamCacheError, TeamProvisioner, TeamSeed, TeamStore, TeamStoreError,
};
use betterlite_auth::auth_user_lookup::cadence::CadenceClock;
use betterlite_auth::auth_user_lookup::ports::PortError;
use betterlite_cache::backend::BackendError;
use betterlite_cache::dual_cache::DualCacheError;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use serde_json::Value;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

type Cached<Model> = Result<Option<Model>, TeamCacheError>;
type Written = Result<(), TeamCacheError>;

// what one step changes and records; the scripted rows below stay fixed for the whole call
pub(crate) struct Board {
    pub(crate) events: Vec<String>,
    pub(crate) logs: Vec<String>,
    pub(crate) fail: Vec<String>,
    pub(crate) now: Duration,
    pub(crate) teams: BTreeMap<String, LiteLLMTeamTableCachedObj>,
    pub(crate) groups: BTreeMap<String, AccessGroupTable>,
}

pub(crate) struct Script {
    pub(crate) board: Mutex<Board>,
    pub(crate) teams: Vec<JsonData>,
    pub(crate) groups: Vec<JsonData>,
    pub(crate) permissions: Vec<JsonData>,
    pub(crate) created: Option<JsonData>,
}

// every collaborator the lookup takes, over one script; handle names the Prisma client it stands for
pub(crate) struct Port {
    pub(crate) script: Arc<Script>,
    pub(crate) handle: &'static str,
}

impl Port {
    // a cache write or a permission load: a scripted failure refuses it before anything is recorded
    fn admit(&self, event: String, failure: &str) -> Option<MutexGuard<'_, Board>> {
        let mut board = self
            .script
            .board
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if board.fail.iter().any(|name| name == failure) {
            return None;
        }
        board.events.push(event);
        Some(board)
    }

    // a table read or new_team: the call is recorded, then refused when the step scripts it to fail
    fn observe(&self, event: String, failure: &str) -> bool {
        let mut board = self
            .script
            .board
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        board.events.push(event);
        board.fail.iter().any(|name| name == failure)
    }

    // one Prisma find on a table: the step fails it by naming the table
    fn select(
        &self,
        table: &str,
        handle: &str,
        rows: &[JsonData],
        (field, value): (&str, &str),
    ) -> StoreAnswer<Vec<JsonData>> {
        if self.observe(format!("db {table} {handle} {value}"), table) {
            return Err(TeamStoreError::Read(Box::new(AuthReadError::Repository(
                DbError::NoDbConnected,
            ))));
        }
        Ok(rows_where(rows, field, value))
    }
}

fn refusal() -> TeamCacheError {
    TeamCacheError::Cache(AuthCacheError::Backend(Box::new(DualCacheError::Memory(
        BackendError::Unsupported("scripted"),
    ))))
}

fn rows_where(rows: &[JsonData], field: &str, value: &str) -> Vec<JsonData> {
    let wanted = JsonData::String(value.to_owned());
    let mut found = Vec::new();
    for row in rows {
        if let JsonData::Object(fields) = row
            && fields.get(field) == Some(&wanted)
        {
            found.push(row.clone());
        }
    }
    found
}

#[async_trait]
impl TeamCache for Port {
    async fn read_team(&self, key: &str) -> Cached<LiteLLMTeamTableCachedObj> {
        let board = self.admit(format!("read {key}"), "").ok_or_else(refusal)?;
        Ok(board.teams.get(key).cloned())
    }

    async fn write_team(&self, key: &str, team: &LiteLLMTeamTableCachedObj, ttl: f64) -> Written {
        let failure = format!("write {key}");
        self.admit(format!("write {key} ttl={ttl}"), &failure)
            .ok_or_else(refusal)?
            .teams
            .insert(key.to_owned(), team.clone());
        Ok(())
    }

    async fn cache_team(&self, team_id: &str, team: &mut LiteLLMTeamTableCachedObj) -> Written {
        let now = self
            .script
            .board
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .now;
        team.last_refreshed_at = Some(now.as_secs_f64());
        self.write_team(&format!("team_id:{team_id}"), team, 60.0_f64)
            .await
    }

    async fn read_access_group(&self, key: &str) -> Cached<AccessGroupTable> {
        let board = self.admit(format!("read {key}"), "").ok_or_else(refusal)?;
        Ok(board.groups.get(key).cloned())
    }

    async fn cache_access_group(&self, access_group_id: &str, table: &AccessGroupTable) -> Written {
        let key = format!("access_group_id:{access_group_id}");
        let failure = format!("write {key}");
        self.admit(format!("write {key} ttl=600"), &failure)
            .ok_or_else(refusal)?
            .groups
            .insert(key, table.clone());
        Ok(())
    }
}

#[async_trait]
impl ObjectPermissionCache for Port {
    async fn read(&self, key: &str) -> PermissionAnswer {
        let admitted = self.admit(format!("read {key}"), "permission");
        admitted.map(|_| None).ok_or_else(|| {
            PermissionCacheError::from(AuthCacheError::from(DualCacheError::Memory(
                BackendError::Unsupported("scripted"),
            )))
        })
    }

    async fn write(
        &self,
        key: &str,
        _permission: &LiteLLMObjectPermissionTable,
        ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        let admitted = self.admit(format!("write {key} ttl={ttl}"), &format!("write {key}"));
        admitted.map(drop).ok_or_else(|| {
            PermissionCacheError::from(AuthCacheError::from(DualCacheError::Memory(
                BackendError::Unsupported("scripted"),
            )))
        })
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

#[async_trait]
impl TeamStore for Port {
    async fn find_team(&self, team_id: &str, use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        let handle = if use_writer { "writer" } else { "reader" };
        let rows = &self.script.teams;
        let mut found = self.select("litellm_teamtable", handle, rows, ("team_id", team_id))?;
        Ok(found.pop())
    }

    async fn find_by_alias(&self, team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        let rows = &self.script.teams;
        self.select(
            "litellm_teamtable",
            "reader",
            rows,
            ("team_alias", team_alias),
        )
    }

    async fn find_access_group(
        &self,
        access_group_id: &str,
        use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>> {
        let handle = if use_writer { "writer" } else { "reader" };
        let filter = ("access_group_id", access_group_id);
        let rows = &self.script.groups;
        let mut found = self.select("litellm_accessgrouptable", handle, rows, filter)?;
        Ok(found.pop())
    }
}

#[async_trait]
impl ObjectPermissionStore for Port {
    async fn find_permission(&self, id: &str) -> Result<Option<JsonData>, PermissionError> {
        let event = format!("db litellm_objectpermissiontable {} {id}", self.handle);
        if self.admit(event, "permission").is_none() {
            return Err(PermissionError::NoDatabase);
        }
        Ok(rows_where(&self.script.permissions, "object_permission_id", id).pop())
    }
}

#[async_trait]
impl TeamProvisioner for Port {
    async fn create(&self, seed: TeamSeed) -> Result<LiteLLMTeamTable, PortError> {
        let principal = serde_json::to_value(&seed.principal).map_err(PortError::from_error)?;
        let role = principal
            .get("user_role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if self.observe(format!("provision {} {role}", seed.team_id), "provision") {
            return Err(PortError::new("the scripted new_team call failed"));
        }
        let created = self
            .script
            .created
            .clone()
            .ok_or_else(|| PortError::new("the case scripts no created team"))?;
        LiteLLMTeamTable::try_from(created).map_err(PortError::from_error)
    }
}

impl CadenceClock for Port {
    fn now(&self) -> Duration {
        self.script
            .board
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .now
    }
}

// the levels auth_team_lookup logged during the step, in order
pub(crate) struct Levels {
    pub(crate) script: Arc<Script>,
}

impl<S: Subscriber> Layer<S> for Levels {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        if metadata
            .target()
            .starts_with("betterlite_auth::auth_team_lookup")
        {
            let mut board = self
                .script
                .board
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            board.logs.push(metadata.level().to_string());
        }
    }
}
