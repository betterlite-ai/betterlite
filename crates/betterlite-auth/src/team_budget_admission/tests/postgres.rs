#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::database_availability::message::TRANSIENT_MESSAGE;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use crate::auth_cache::{AuthCache, gateway_cache};
use crate::auth_end_users::defaults::DefaultBudgetLookup;
use crate::auth_end_users::postgres::DatabaseEndUsers;
use crate::auth_errors::conversion::as_proxy_exception;
use crate::auth_invalidation::registry::MembershipInflight;
use crate::auth_request_models::RequestedModels;
use crate::membership_load::MembershipLookup;
use crate::membership_load::cache::MembershipCache;
use crate::membership_load::error::{MembershipLoadError, MembershipReadFailure};
use crate::membership_load::index::SharedMembershipIndex;
use crate::membership_load::rows::{DatabaseTeamMembership, TeamMembershipRows as _};

use super::super::error::{TeamBudgetError, membership_failure};
use super::super::sources::Membership;
use super::{DEFAULT_DEPTH, Harness, admit_with, key, money, request, team};

// the member override is zero and the second member has no budget row, so the team default applies to them
const SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, created_by, updated_by)
    VALUES ('member-zero', 0.0, 'tester', 'tester'),
           ('team-default', 7.0, 'tester', 'tester');
INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend, budget_id)
    VALUES ('user-1', 'team-1', 2.0, 'member-zero'),
           ('user-2', 'team-1', 1.5, NULL);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["team-budget-admission-postgres-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the database fixture");
    let pool = Pool::builder(Manager::new(fixture.options.config.clone(), NoTls))
        .max_size(2)
        .build()
        .expect("the database pool");
    let client = pool.get().await.expect("the database connection");
    client.batch_execute(SEED).await.expect("the seeded rows");
    (fixture, pool)
}

fn store(pool: &Pool) -> Arc<PostgresStore> {
    Arc::new(PostgresStore {
        pool: Some(pool.clone()),
    })
}

fn tracker() -> Arc<DbLookupStallTracker<SystemClock>> {
    Arc::new(DbLookupStallTracker {
        clock: SystemClock,
        last_hit: Mutex::new(None),
    })
}

fn memory() -> Arc<LocalMemoryOps> {
    Arc::new(LocalMemoryOps::new(1024, Arc::new(|| 0.0_f64)).expect("the local memory tier"))
}

// B2318's shared load and B739's default lookup over the stored rows, behind one local cache
struct Stored {
    memberships: MembershipLookup,
    budget_rows: DatabaseEndUsers,
    cache: Arc<StoredCache>,
}

type StoredCache = AuthCache<DualCache<LocalMemoryOps>>;

fn membership_rows(pool: &Pool) -> DatabaseTeamMembership {
    DatabaseTeamMembership {
        writer: store(pool),
        reader: store(pool),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: tracker(),
        tasks: TaskTracker::new(),
    }
}

fn stored(pool: &Pool) -> Stored {
    let settings = DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    };
    let cache = Arc::new(gateway_cache(memory(), memory(), None, settings, 45.0_f64));
    let membership_cache: Arc<dyn MembershipCache> = Arc::<StoredCache>::clone(&cache);
    let memberships = MembershipLookup {
        cache: membership_cache,
        rows: Some(Arc::new(membership_rows(pool))),
        index: Arc::new(SharedMembershipIndex::new(Arc::new(
            MembershipInflight::default(),
        ))),
        tasks: TaskTracker::new(),
    };
    let budget_rows = DatabaseEndUsers {
        writer: store(pool),
        reader: store(pool),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: tracker(),
        tasks: TaskTracker::new(),
    };
    Stored {
        memberships,
        budget_rows,
        cache,
    }
}

#[rstest]
#[case::zero_override_denies("user-1", "2", "0", Some("0"))]
#[case::positive_default_caps("user-2", "1.5", "6.99", None)]
#[tokio::test]
async fn stored_member_budgets_decide_admission(
    #[case] user_id: &str,
    #[case] fallback: &str,
    #[case] spend: &str,
    #[case] denied_at: Option<&str>,
) {
    let (fixture, pool) = database().await;
    let rows = stored(&pool);
    let defaults = DefaultBudgetLookup {
        rows: Some(&rows.budget_rows),
        cache: rows.cache.as_ref(),
        max_end_user_budget_id: None,
    };
    // the spend counter and the clock are pinned apart from the stored rows
    let harness = Harness::spending(&[spend]);
    let row = team(r#"{"team_id":"team-1","metadata":{"team_member_budget_id":"team-default"}}"#);
    let token = key(&format!(
        r#"{{"token":"sk-stored","user_id":"{user_id}","team_id":"team-1"}}"#
    ));
    let model = RequestedModels::None;
    let checked = request(Some(&row), Some(&token), &model, Membership::Unloaded);
    let result = admit_with(
        &harness,
        &rows.memberships,
        &defaults,
        &checked,
        DEFAULT_DEPTH,
    )
    .await;
    let reads = harness.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(
        reads[0].counter_key,
        format!("spend:team_member:{user_id}:team-1")
    );
    assert_eq!(reads[0].fallback, money(fallback));
    match (denied_at, result) {
        (Some(maximum), Err(TeamBudgetError::Denied(denial))) => {
            assert_eq!(denial.max_budget, money(maximum));
            assert_eq!(
                denial.entity_id.unwrap().expose_secret(),
                format!("{user_id}:team-1")
            );
        }
        (None, Ok(_)) => assert_eq!(reads[0].max_budget, Some(money("7"))),
        (_, other) => panic!("unexpected admission for {user_id}: {other:?}"),
    }
    rows.memberships.tasks.close();
    rows.memberships.tasks.wait().await;
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

// Postgres has no SELECT trigger, so the membership table becomes a view whose predicate raises the scripted message
fn view_raising(message: &str) -> String {
    format!(
        r#"
ALTER TABLE "LiteLLM_TeamMembership" RENAME TO "LiteLLM_TeamMembership_base";
CREATE FUNCTION raise_membership_select_error() RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '{message}';
END
$$;
CREATE VIEW "LiteLLM_TeamMembership" AS
    SELECT * FROM "LiteLLM_TeamMembership_base" WHERE raise_membership_select_error()"#
    )
}

async fn raise_on_membership(pool: &Pool, message: &str) {
    let client = pool.get().await.expect("the setup connection");
    client
        .batch_execute(&view_raising(message))
        .await
        .expect("the raising view");
}

async fn read_failure(rows: &DatabaseTeamMembership) -> MembershipReadFailure {
    let Err(failure) = rows.find_membership("user-1", "team-1").await else {
        panic!("the raising view fails the membership read");
    };
    failure
}

fn assert_service_unavailable(error: &MembershipLoadError) {
    let projected = as_proxy_exception(&membership_failure(error));
    let exception = projected.exception();
    assert_eq!(exception.error_type, "no_db_connection");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "503");
    assert_eq!(exception.message, TRANSIENT_MESSAGE);
}

fn assert_unauthorized(error: &MembershipLoadError) {
    let projected = as_proxy_exception(&membership_failure(error));
    let exception = projected.exception();
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.code, "401");
}

#[tokio::test]
async fn stale_plan_membership_failures_answer_service_unavailable() {
    let (fixture, pool) = database().await;
    let rows = membership_rows(&pool);
    raise_on_membership(&pool, "cached plan must not change result type").await;
    let direct = MembershipLoadError::Read(read_failure(&rows).await);
    assert_service_unavailable(&direct);
    assert_service_unavailable(&MembershipLoadError::Shared(Arc::new(direct)));
    rows.tasks.close();
    rows.tasks.wait().await;
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

#[tokio::test]
async fn ordinary_membership_read_errors_stay_unauthorized() {
    let (fixture, pool) = database().await;
    let rows = membership_rows(&pool);
    raise_on_membership(&pool, "membership read rejected").await;
    let direct = MembershipLoadError::Read(read_failure(&rows).await);
    assert_unauthorized(&direct);
    assert_unauthorized(&MembershipLoadError::Shared(Arc::new(direct)));
    rows.tasks.close();
    rows.tasks.wait().await;
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}
