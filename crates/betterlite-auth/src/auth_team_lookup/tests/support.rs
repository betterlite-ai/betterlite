use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::backend::BackendError;
use betterlite_cache::dual_cache::DualCacheError;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::auth_cache::AuthCacheError;
use crate::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use crate::auth_prefetch::permission::store::ObjectPermissionStore;
use crate::auth_read::table::AuthReadError;
use crate::auth_team_lookup::error::{Detail, TeamLookupError};
use crate::auth_team_lookup::lookup::{TeamDatabase, TeamLookup, TeamRequest, TeamSource};
use crate::auth_team_lookup::ports::{
    StoreAnswer, TeamCache, TeamCacheError, TeamProvisioner, TeamSeed, TeamStore, TeamStoreError,
};
use crate::auth_user_lookup::cadence::{Cadence, CadenceClock};
use crate::auth_user_lookup::ports::PortError;

#[derive(Default)]
pub(crate) struct State {
    pub(crate) events: Vec<String>,
    pub(crate) teams: BTreeMap<String, LiteLLMTeamTableCachedObj>,
    pub(crate) groups: BTreeMap<String, AccessGroupTable>,
    pub(crate) team_rows: Vec<JsonData>,
    pub(crate) group_rows: Vec<JsonData>,
    pub(crate) permission_rows: Vec<JsonData>,
    pub(crate) created: Option<JsonData>,
    pub(crate) failing: BTreeSet<String>,
    pub(crate) now: Duration,
}

// every collaborator of the lookup over one recorded state, so a test reads the whole call order
#[derive(Default)]
pub(crate) struct World {
    state: Mutex<State>,
}

impl World {
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, event: String) {
        self.lock().events.push(event);
    }

    fn fails(&self, name: &str) -> bool {
        self.lock().failing.contains(name)
    }

    // a scripted write failure stops before the write is recorded
    fn accept_write(&self, key: &str, ttl: f64) -> Result<(), TeamCacheError> {
        if self.fails(&format!("write {key}")) {
            return Err(cache_failure());
        }
        self.record(format!("write {key} ttl={ttl}"));
        Ok(())
    }

    pub(crate) fn events(&self) -> Vec<String> {
        self.lock().events.clone()
    }

    pub(crate) fn at(&self, seconds: u64) {
        self.lock().now = Duration::from_secs(seconds);
    }
}

pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn team_row(team_id: &str, team_alias: &str, permission_id: Option<&str>) -> JsonData {
    let permission = permission_id.map_or_else(|| "null".to_owned(), |id| format!("\"{id}\""));
    json(&format!(
        r#"{{"team_id": "{team_id}", "team_alias": "{team_alias}", "models": ["gpt-4o"], "object_permission_id": {permission}}}"#
    ))
}

pub(crate) fn cached_team(team_id: &str) -> LiteLLMTeamTableCachedObj {
    LiteLLMTeamTableCachedObj::try_from(team_row(team_id, "cached", None)).unwrap()
}

pub(crate) fn world() -> Arc<World> {
    let world = Arc::new(World::default());
    world.at(100);
    world
}

pub(crate) fn lookup(world: &Arc<World>, in_memory_ttl: i32) -> TeamLookup {
    let clock: Arc<dyn CadenceClock> = Arc::<World>::clone(world);
    let cache: Arc<dyn TeamCache> = Arc::<World>::clone(world);
    let permission_cache: Arc<dyn ObjectPermissionCache> = Arc::<World>::clone(world);
    let store: Arc<dyn TeamStore> = Arc::<World>::clone(world);
    let provisioner: Arc<dyn TeamProvisioner> = Arc::<World>::clone(world);
    let reader = Handle {
        world: Arc::clone(world),
        writer: false,
    };
    let writer = Handle {
        world: Arc::clone(world),
        writer: true,
    };
    TeamLookup {
        cache,
        permission_cache,
        database: Some(TeamDatabase {
            store,
            permission_reader: Arc::new(reader),
            permission_writer: Arc::new(writer),
        }),
        provisioner: Some(provisioner),
        cadence: Arc::new(Cadence::new(100, clock)),
        in_memory_ttl,
    }
}

// a lookup whose prisma_client is None
pub(crate) fn without_database(world: &Arc<World>) -> TeamLookup {
    let mut unwired = lookup(world, 60);
    unwired.database = None;
    unwired
}

pub(crate) const fn request(source: TeamSource) -> TeamRequest<'static> {
    TeamRequest {
        team_id: "t1",
        source,
        team_id_upsert: false,
    }
}

pub(crate) fn assert_http(error: &TeamLookupError, status: u16, detail: Detail) {
    assert_eq!(error.http(), Some((status, detail)));
}

fn cache_failure() -> TeamCacheError {
    TeamCacheError::Cache(AuthCacheError::Backend(Box::new(DualCacheError::Memory(
        BackendError::Unsupported("scripted"),
    ))))
}

fn matching(rows: &[JsonData], field: &str, value: &str) -> Vec<JsonData> {
    let wanted = JsonData::String(value.to_owned());
    rows.iter()
        .filter(|row| matches!(row, JsonData::Object(fields) if fields.get(field) == Some(&wanted)))
        .cloned()
        .collect()
}

#[async_trait]
impl TeamCache for World {
    async fn read_team(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        self.record(format!("read {key}"));
        Ok(self.lock().teams.get(key).cloned())
    }

    async fn write_team(
        &self,
        key: &str,
        team: &LiteLLMTeamTableCachedObj,
        ttl: f64,
    ) -> Result<(), TeamCacheError> {
        self.accept_write(key, ttl)?;
        self.lock().teams.insert(key.to_owned(), team.clone());
        Ok(())
    }

    async fn cache_team(
        &self,
        team_id: &str,
        team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), TeamCacheError> {
        team.last_refreshed_at = Some(1.5_f64);
        self.write_team(&format!("team_id:{team_id}"), team, 60.0_f64)
            .await
    }

    async fn read_access_group(
        &self,
        key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        self.record(format!("read {key}"));
        Ok(self.lock().groups.get(key).cloned())
    }

    async fn cache_access_group(
        &self,
        access_group_id: &str,
        table: &AccessGroupTable,
    ) -> Result<(), TeamCacheError> {
        let key = format!("access_group_id:{access_group_id}");
        self.accept_write(&key, 600.0_f64)?;
        self.lock().groups.insert(key, table.clone());
        Ok(())
    }
}

#[async_trait]
impl ObjectPermissionCache for World {
    async fn read(&self, key: &str) -> PermissionAnswer {
        if self.fails("permission") {
            return Err(PermissionCacheError::from(AuthCacheError::Backend(
                Box::new(DualCacheError::Memory(BackendError::Unsupported(
                    "permission",
                ))),
            )));
        }
        self.record(format!("read {key}"));
        Ok(None)
    }

    async fn write(
        &self,
        key: &str,
        _permission: &LiteLLMObjectPermissionTable,
        ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        self.record(format!("write {key} ttl={ttl}"));
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

fn store_failure() -> TeamStoreError {
    TeamStoreError::Read(Box::new(AuthReadError::Repository(DbError::NoDbConnected)))
}

#[async_trait]
impl TeamStore for World {
    async fn find_team(&self, team_id: &str, use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        self.record(format!("team {team_id} writer={use_writer}"));
        if self.fails("team") {
            return Err(store_failure());
        }
        Ok(matching(&self.lock().team_rows, "team_id", team_id).pop())
    }

    async fn find_by_alias(&self, team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        self.record(format!("alias {team_alias}"));
        if self.fails("team") {
            return Err(store_failure());
        }
        Ok(matching(&self.lock().team_rows, "team_alias", team_alias))
    }

    async fn find_access_group(
        &self,
        access_group_id: &str,
        use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>> {
        self.record(format!("access {access_group_id} writer={use_writer}"));
        if self.fails("access") {
            return Err(store_failure());
        }
        Ok(matching(&self.lock().group_rows, "access_group_id", access_group_id).pop())
    }
}

#[async_trait]
impl TeamProvisioner for World {
    async fn create(&self, seed: TeamSeed) -> Result<LiteLLMTeamTable, PortError> {
        let granted = seed.principal.extra.user_role.as_ref();
        let role = granted.map_or_else(String::new, ToString::to_string);
        self.record(format!("provision {} {role}", seed.team_id));
        if self.fails("provision") {
            return Err(PortError::new("the team was not created"));
        }
        let created = self.lock().created.clone().unwrap();
        Ok(LiteLLMTeamTable::try_from(created).unwrap())
    }
}

impl CadenceClock for World {
    fn now(&self) -> Duration {
        self.lock().now
    }
}

struct Handle {
    world: Arc<World>,
    writer: bool,
}

#[async_trait]
impl ObjectPermissionStore for Handle {
    async fn find_permission(&self, id: &str) -> Result<Option<JsonData>, PermissionError> {
        self.world
            .record(format!("permission {id} writer={}", self.writer));
        let rows = self.world.lock().permission_rows.clone();
        Ok(matching(&rows, "object_permission_id", id).pop())
    }
}

// the levels this module logged, in order
#[derive(Clone, Default)]
pub(crate) struct Levels {
    seen: Arc<Mutex<Vec<Level>>>,
}

impl Levels {
    pub(crate) fn install(&self) -> DefaultGuard {
        set_default(tracing_subscriber::registry().with(self.clone()))
    }

    pub(crate) fn seen(&self) -> Vec<Level> {
        self.seen.lock().unwrap().clone()
    }
}

impl<S: Subscriber> Layer<S> for Levels {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if event
            .metadata()
            .target()
            .starts_with("betterlite_auth::auth_team_lookup")
        {
            self.seen.lock().unwrap().push(*event.metadata().level());
        }
    }
}
