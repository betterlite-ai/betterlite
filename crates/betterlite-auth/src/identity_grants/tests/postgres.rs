use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::member::MemberRole;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use crate::auth_cache::{AuthCache, gateway_cache};
use crate::auth_invalidation::AuthInvalidation;
use crate::auth_invalidation::clock::SystemClock as InvalidationClock;
use crate::auth_invalidation::ports::{AuthInvalidationCache, SpendCounterCache};
use crate::auth_invalidation::registry::MembershipInflight;
use crate::auth_invalidation::transport::InvalidationTransport;
use crate::auth_prefetch::permission::store::{BoundedPermissionStore, PermissionTable};
use crate::auth_pubsub::eviction::LocalEviction;
use crate::auth_read::table::ReadContext;
use crate::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use crate::auth_team_lookup::ports::GatewayTeamCache;
use crate::auth_team_lookup::store::DatabaseTeamStore;
use crate::auth_user_lookup::cadence::{Cadence, WallClock};
use crate::auth_user_lookup::lookup::UserLookup;
use crate::auth_user_lookup::ports::{BudgetResetCalculator, NoDefaultTeams, PortError};
use crate::auth_user_lookup::store::DatabaseUserStore;
use crate::identity_grants::records::{GrantDenial, GrantOutcome, UserQuery};
use crate::identity_grants::resolver::GrantResolver;
use crate::identity_grants::team_grants::team_grants;
use crate::membership_load::MembershipLookup;
use crate::membership_load::index::SharedMembershipIndex;
use crate::membership_load::rows::DatabaseTeamMembership;

type Cache = AuthCache<DualCache<LocalMemoryOps>>;

const SEED: &str = r#"
INSERT INTO "LiteLLM_UserTable" (user_id, user_email, teams) VALUES
    ('legacy-id', 'legacy@example.com', ARRAY['team-1']),
    ('outsider', NULL, ARRAY['team-2']),
    ('orphan', NULL, ARRAY['team-gone']);
INSERT INTO "LiteLLM_ModelTable" (id, aliases, created_by, updated_by)
    VALUES (7, '{"fast": "gpt-4o"}', 'a', 'a');
INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, model_id, members_with_roles)
    VALUES ('team-1', 'Team One', 7, '[{"user_id": "legacy-id", "role": "admin"}]');
INSERT INTO "LiteLLM_BudgetTable" (budget_id, tpm_limit, rpm_limit, created_by, updated_by)
    VALUES ('budget-1', 11, 7, 'a', 'a');
INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend, budget_id)
    VALUES ('legacy-id', 'team-1', 2.5, 'budget-1');
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Silent;

#[async_trait]
impl InvalidationTransport for Silent {
    async fn publish(&self, _cache_key: &str, _new_value: Option<f64>, _ttl: Option<f64>) {}

    async fn evict_and_broadcast(&self, _cache_keys: &[String], _local: &dyn LocalEviction) {}
}

struct NoReset;

impl BudgetResetCalculator for NoReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new("no seeded user carries a budget duration"))
    }
}

struct Readers {
    users: UserLookup,
    teams: TeamLookup,
    memberships: MembershipLookup,
}

fn memory() -> Arc<LocalMemoryOps> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 100.0_f64);
    Arc::new(LocalMemoryOps::new(1024, clock).unwrap())
}

const fn settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    }
}

fn tracker() -> Arc<DbLookupStallTracker<SystemClock>> {
    Arc::new(DbLookupStallTracker {
        clock: SystemClock,
        last_hit: Mutex::new(None),
    })
}

// The three tables the readers query, all on the one seeded database.
fn stores(
    store: &PostgresStore,
    tasks: &TaskTracker,
) -> (DatabaseUserStore, DatabaseTeamStore, DatabaseTeamMembership) {
    let users = DatabaseUserStore {
        reader: store.clone(),
        writer: store.clone(),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: tracker(),
        tasks: tasks.clone(),
    };
    let teams = DatabaseTeamStore {
        reader: store.clone(),
        writer: store.clone(),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: tracker(),
        tasks: tasks.clone(),
    };
    let members = DatabaseTeamMembership {
        writer: Arc::new(store.clone()),
        reader: Arc::new(store.clone()),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: tracker(),
        tasks: tasks.clone(),
    };
    (users, teams, members)
}

fn identity_readers(
    users: DatabaseUserStore,
    members: DatabaseTeamMembership,
    cache: &Arc<Cache>,
    tasks: &TaskTracker,
) -> (UserLookup, MembershipLookup) {
    let user_reader = UserLookup {
        cache: Arc::<Cache>::clone(cache),
        store: Some(Arc::new(users)),
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: Arc::new(NoReset),
        enrollment: Arc::new(NoDefaultTeams),
        tasks: tasks.clone(),
        defaults: None,
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    };
    let registry = Arc::new(MembershipInflight::default());
    let membership_reader = MembershipLookup {
        cache: Arc::<Cache>::clone(cache),
        rows: Some(Arc::new(members)),
        index: Arc::new(SharedMembershipIndex::new(registry)),
        tasks: tasks.clone(),
    };
    (user_reader, membership_reader)
}

fn permissions(
    store: &PostgresStore,
    use_writer: bool,
    tasks: &TaskTracker,
) -> Arc<BoundedPermissionStore<PermissionTable>> {
    let table = PermissionTable {
        reader: store.clone(),
        writer: store.clone(),
        availability: Arc::new(Availability::default()),
        use_writer,
    };
    let context = ReadContext {
        default_deadline_seconds: 5.0_f64,
        tracker: tracker(),
        tasks: tasks.clone(),
    };
    Arc::new(BoundedPermissionStore {
        table: Arc::new(table),
        context,
    })
}

fn team_lookup(
    teams: DatabaseTeamStore,
    store: &PostgresStore,
    cache: &Arc<Cache>,
    tasks: &TaskTracker,
) -> TeamLookup {
    let auth: Arc<dyn AuthInvalidationCache> = Arc::<Cache>::clone(cache);
    let spend: Arc<dyn SpendCounterCache> = Arc::new(DualCache::new(memory(), None, settings()));
    let invalidation = AuthInvalidation {
        auth,
        usage: None,
        spend,
        transport: Arc::new(Silent),
        clock: Arc::new(InvalidationClock),
        access_group_ttl: 600.0_f64,
    };
    TeamLookup {
        cache: Arc::new(GatewayTeamCache {
            cache: Arc::<Cache>::clone(cache),
            invalidation: Arc::new(invalidation),
        }),
        permission_cache: Arc::<Cache>::clone(cache),
        database: Some(TeamDatabase {
            store: Arc::new(teams),
            permission_reader: permissions(store, false, tasks),
            permission_writer: permissions(store, true, tasks),
        }),
        provisioner: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        in_memory_ttl: 60,
    }
}

async fn denial(readers: &Readers, user_id: &str, team_id: &str) -> GrantDenial {
    let resolver = GrantResolver {
        users: &readers.users,
        teams: &readers.teams,
        memberships: &readers.memberships,
    };
    let caller = UserQuery {
        user_id: Some(user_id.to_owned()),
        user_email: None,
        sso_user_id: None,
        upsert: false,
    };
    match resolver.resolve(&caller, Some(team_id)).await {
        GrantOutcome::Denied(denied) => denied,
        other @ (GrantOutcome::Resolved(_) | GrantOutcome::Degraded(_)) => {
            panic!("{user_id} in {team_id} is denied: {other:?}")
        }
    }
}

// A configured POSTGRES_TEST_URL that fails to connect fails the test instead of starting a container.
async fn seeded() -> (PostgresFixture, Pool, Readers, TaskTracker) {
    let url = Environment::parse_from(["identity-grants-postgres-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).unwrap();
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .unwrap();
    let pool = Pool::builder(Manager::new(fixture.options.config.clone(), NoTls))
        .max_size(4)
        .build()
        .unwrap();
    pool.get().await.unwrap().batch_execute(SEED).await.unwrap();
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let cache = Arc::new(gateway_cache(
        memory(),
        memory(),
        None,
        settings(),
        60.0_f64,
    ));
    let tasks = TaskTracker::new();
    let (user_store, team_store, member_rows) = stores(&store, &tasks);
    let (users, memberships) = identity_readers(user_store, member_rows, &cache, &tasks);
    let readers = Readers {
        users,
        teams: team_lookup(team_store, &store, &cache, &tasks),
        memberships,
    };
    (fixture, pool, readers, tasks)
}

async fn close(fixture: PostgresFixture, pool: Pool, tasks: TaskTracker) {
    tasks.close();
    tasks.wait().await;
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn real_readers_rebind_before_membership() {
    let (fixture, pool, readers, tasks) = seeded().await;
    let resolver = GrantResolver {
        users: &readers.users,
        teams: &readers.teams,
        memberships: &readers.memberships,
    };
    let by_email = UserQuery {
        user_id: Some("new-id".to_owned()),
        user_email: Some("legacy@example.com".to_owned()),
        sso_user_id: None,
        upsert: false,
    };
    let GrantOutcome::Resolved(grants) = resolver.resolve(&by_email, Some("team-1")).await else {
        panic!("the email match resolves the legacy row's grants");
    };
    let effective = grants.identity.effective_user_id.as_deref();
    assert_eq!(effective, Some("legacy-id"));
    let membership = grants.identity.membership.as_ref().unwrap();
    assert_eq!(membership.user_id, "legacy-id");
    let team = &grants.team.as_ref().unwrap().team;
    let projected = team_grants(team, Some(membership), effective);
    assert_eq!(projected.team_alias.as_deref(), Some("Team One"));
    let fast = projected.team_model_aliases.unwrap().get("fast").cloned();
    assert_eq!(fast.as_deref(), Some("gpt-4o"));
    assert_eq!(projected.team_member.unwrap().role, MemberRole::Admin);
    assert_eq!(projected.team_member_spend, Some(Decimal::new(25, 1)));
    assert_eq!(projected.team_member_tpm_limit, Some(11_i64));
    assert_eq!(projected.team_member_rpm_limit, Some(7_i64));
    close(fixture, pool, tasks).await;
}

#[tokio::test]
async fn real_readers_deny_by_phase() {
    let (fixture, pool, readers, tasks) = seeded().await;
    let gone = GrantDenial::UserGone {
        user_id: "nobody".to_owned(),
    };
    assert_eq!(denial(&readers, "nobody", "team-1").await, gone);
    let outsider = GrantDenial::NotAMember {
        user_id: "outsider".to_owned(),
        team_id: "team-1".to_owned(),
    };
    assert_eq!(denial(&readers, "outsider", "team-1").await, outsider);
    let missing = GrantDenial::TeamGone {
        team_id: "team-gone".to_owned(),
    };
    assert_eq!(denial(&readers, "orphan", "team-gone").await, missing);
    close(fixture, pool, tasks).await;
}
