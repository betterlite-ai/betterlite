use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError, PermissionError,
};
use betterlite_auth::auth_prefetch::permission::store::ObjectPermissionStore;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use betterlite_auth::auth_team_lookup::ports::{StoreAnswer, TeamCache, TeamCacheError, TeamStore};
use betterlite_auth::auth_user_lookup::cache::{UserCache, UserCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, NoDefaultTeams, PortError};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use tokio::sync::RwLock;
use tokio_util::task::TaskTracker;

use super::{FakeReads, names};
use crate::auth::object_permission_hydration::reads::ObjectPermissionReads as _;
use crate::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use crate::server_grants::db_reads::DbGrantReads;
use crate::server_grants::error::GrantReadError;
use crate::server_grants::{GrantReads as _, RegistryGrantReads as _};
use crate::server_record::MCPServer;

const CACHED_TEAM: &str = r#"{"team_id":"team-1","blocked":true,"access_group_ids":["ag-1","ag-2"],"object_permission":{"object_permission_id":"op-team"}}"#;
const CACHED_USER: &str = r#"{"user_id":"user-1","teams":["team-b","team-a"]}"#;
const LEGACY_SERVERS: &str = r#"[{"server_id":"srv-legacy","name":"legacy","alias":null,"server_name":null,"transport":"http","access_groups":["grp-1"]},{"server_id":"srv-other","name":"other","alias":null,"server_name":null,"transport":"http","access_groups":["grp-2"]}]"#;

// the team, access group and user caches behind B737 and B736; the tables behind them hold no rows
#[derive(Default)]
struct World {
    team: Option<LiteLLMTeamTableCachedObj>,
    access_groups: BTreeMap<String, AccessGroupTable>,
    user: Option<LiteLLMUserTable>,
    team_reads: Mutex<Vec<bool>>,
    access_group_reads: Mutex<Vec<bool>>,
}

struct NoReset;

impl BudgetResetCalculator for NoReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new("a grant read never computes a budget reset"))
    }
}

#[async_trait]
impl TeamCache for World {
    async fn read_team(
        &self,
        _key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
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
        key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        Ok(self.access_groups.get(key).cloned())
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
impl TeamStore for World {
    async fn find_team(&self, _team_id: &str, use_writer: bool) -> StoreAnswer<Option<JsonData>> {
        self.team_reads.lock().unwrap().push(use_writer);
        Ok(None)
    }

    async fn find_by_alias(&self, _team_alias: &str) -> StoreAnswer<Vec<JsonData>> {
        Ok(Vec::new())
    }

    async fn find_access_group(
        &self,
        _access_group_id: &str,
        use_writer: bool,
    ) -> StoreAnswer<Option<JsonData>> {
        self.access_group_reads.lock().unwrap().push(use_writer);
        Ok(None)
    }
}

#[async_trait]
impl ObjectPermissionCache for World {
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
        60.0_f64
    }
}

#[async_trait]
impl ObjectPermissionStore for World {
    async fn find_permission(&self, _id: &str) -> Result<Option<JsonData>, PermissionError> {
        Ok(None)
    }
}

#[async_trait]
impl UserCache for World {
    async fn read(&self, _key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        Ok(self.user.clone())
    }

    async fn write(
        &self,
        _key: &str,
        _user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

// the lookups DbGrantReads borrows, built over one world; the user lookup has no table, as without a database
struct Fixture {
    world: Arc<World>,
    teams: TeamLookup,
    users: UserLookup,
    servers: SharedRegistry,
}

fn team_lookup(world: &Arc<World>) -> TeamLookup {
    TeamLookup {
        cache: Arc::<World>::clone(world),
        permission_cache: Arc::<World>::clone(world),
        database: Some(TeamDatabase {
            store: Arc::<World>::clone(world),
            permission_reader: Arc::<World>::clone(world),
            permission_writer: Arc::<World>::clone(world),
        }),
        provisioner: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        in_memory_ttl: 60,
    }
}

fn user_lookup(world: &Arc<World>) -> UserLookup {
    UserLookup {
        cache: Arc::<World>::clone(world),
        store: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: Arc::new(NoReset),
        enrollment: Arc::new(NoDefaultTeams),
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    }
}

impl Fixture {
    fn new(world: World) -> Self {
        let shared = Arc::new(world);
        let settings = RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: true,
            internal_ip_ranges: None,
        };
        let config_servers: Vec<MCPServer> = serde_json::from_str(LEGACY_SERVERS).unwrap();
        let registry = ServerRegistry::new(settings, config_servers, Vec::new());
        Self {
            teams: team_lookup(&shared),
            users: user_lookup(&shared),
            world: shared,
            servers: Arc::new(RwLock::new(registry)),
        }
    }

    fn reads(&self) -> DbGrantReads<'_, FakeReads, FakeReads> {
        DbGrantReads {
            permissions: FakeReads::standard(),
            registry: FakeReads::standard(),
            teams: &self.teams,
            users: &self.users,
            servers: &self.servers,
            server_rows: None,
        }
    }
}

fn access_group(access_group_id: &str, server_ids: &[&str]) -> (String, AccessGroupTable) {
    // a list of &str prints as a JSON array of strings
    let table = format!(
        r#"{{"access_group_id":"{access_group_id}","access_group_name":"{access_group_id}","access_mcp_server_ids":{server_ids:?}}}"#
    );
    (
        format!("access_group_id:{access_group_id}"),
        serde_json::from_str(&table).unwrap(),
    )
}

#[tokio::test]
async fn db_reads_hand_out_their_permission_and_registry_owners() {
    let fixture = Fixture::new(World::default());
    let mut reads = fixture.reads();
    reads.permissions.database = false;
    reads.registry.registry_ids = names(&["srv-only"]);
    assert!(!reads.permissions().has_database());
    assert_eq!(
        reads.registry().registry_server_ids().await,
        names(&["srv-only"])
    );
}

#[tokio::test]
async fn db_reads_team_comes_from_the_cached_row() {
    let cached = serde_json::from_str::<JsonData>(CACHED_TEAM).unwrap();
    let fixture = Fixture::new(World {
        team: Some(LiteLLMTeamTableCachedObj::try_from(cached).unwrap()),
        ..World::default()
    });
    let record = fixture
        .reads()
        .team("team-1", false)
        .await
        .unwrap()
        .unwrap();
    assert!(record.blocked);
    assert_eq!(record.access_group_ids, names(&["ag-1", "ag-2"]));
    assert_eq!(
        record
            .object_permission
            .map(|permission| permission.object_permission_id),
        Some(String::from("op-team"))
    );
    assert!(fixture.world.team_reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn db_reads_fresh_team_reads_the_writer_and_misses() {
    let cached = serde_json::from_str::<JsonData>(CACHED_TEAM).unwrap();
    let fixture = Fixture::new(World {
        team: Some(LiteLLMTeamTableCachedObj::try_from(cached).unwrap()),
        ..World::default()
    });
    let error = fixture.reads().team("team-1", true).await.unwrap_err();
    assert!(matches!(
        error,
        GrantReadError::Team(TeamLookupError::TeamNotFound { team_id }) if team_id == "team-1"
    ));
    assert_eq!(*fixture.world.team_reads.lock().unwrap(), Vec::from([true]));
}

#[tokio::test]
async fn db_reads_user_teams_come_from_the_cached_user() {
    let fixture = Fixture::new(World {
        user: Some(serde_json::from_str(CACHED_USER).unwrap()),
        ..World::default()
    });
    let reads = fixture.reads();
    assert_eq!(
        reads.user_team_ids("user-1", false).await.unwrap(),
        Some(names(&["team-b", "team-a"]))
    );
    let error = reads.user_team_ids("user-1", true).await.unwrap_err();
    assert!(matches!(
        error,
        GrantReadError::User(UserLookupError::NoDatabase)
    ));
}

#[tokio::test]
async fn db_reads_access_groups_skip_a_group_that_does_not_load() {
    let fixture = Fixture::new(World {
        access_groups: BTreeMap::from([
            access_group("ag-1", &["srv-a", "srv-b"]),
            access_group("ag-2", &["srv-b", "srv-c"]),
        ]),
        ..World::default()
    });
    let requested = names(&["ag-1", "ag-missing", "ag-2"]);
    assert_eq!(
        fixture
            .reads()
            .access_group_server_ids(&requested, false)
            .await
            .unwrap(),
        names(&["srv-a", "srv-b", "srv-c"])
    );
    assert_eq!(
        *fixture.world.access_group_reads.lock().unwrap(),
        Vec::from([false])
    );
}

#[tokio::test]
async fn db_reads_fresh_access_group_fault_is_returned() {
    let fixture = Fixture::new(World {
        access_groups: BTreeMap::from([access_group("ag-1", &["srv-a"])]),
        ..World::default()
    });
    let error = fixture
        .reads()
        .access_group_server_ids(&names(&["ag-1"]), true)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        GrantReadError::AccessGroup(TeamLookupError::AccessGroupNotFound { access_group_id })
            if access_group_id == "ag-1"
    ));
    assert_eq!(
        *fixture.world.access_group_reads.lock().unwrap(),
        Vec::from([true])
    );
}

#[tokio::test]
async fn db_reads_legacy_access_groups_match_the_registry() {
    let fixture = Fixture::new(World::default());
    assert_eq!(
        fixture
            .reads()
            .servers_from_access_groups(&names(&["grp-1"]), false)
            .await
            .unwrap(),
        names(&["srv-legacy"])
    );
}
