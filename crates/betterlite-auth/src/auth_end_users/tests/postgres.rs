#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_config::sections::end_user_registry::Settings;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use super::super::check::{EndUserBudgetError, check_end_user_budget};
use super::super::object::{EndUserObjects, EndUserRequest};
use super::super::postgres::DatabaseEndUsers;
use super::super::rows::{EndUserFetchError, EndUserRows as _};
use super::support::{TestCache, money, reseed_over, spend_dual_cache, test_cache};
use crate::auth_read::table::AuthReadError;
use crate::current_spend::CurrentSpend;
use crate::end_user_registry::EndUserRegistry;
use crate::spend_reads::batch::NoSpendBatch;
use crate::spend_reseed::cache::ReseedCache as _;
use crate::spend_reseed::sql::SqlSpendCounterRows;

// LiteLLM_EndUserTable's budget_id foreign key admits no dangling id, so a live row always carries its relation
const SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, created_by, updated_by)
    VALUES ('budget-1', 100.0, 'tester', 'tester');
INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id)
    VALUES ('permission-1');
INSERT INTO "LiteLLM_EndUserTable" (user_id, spend, budget_id)
    VALUES ('with-budget', 5.0, 'budget-1');
INSERT INTO "LiteLLM_EndUserTable" (user_id, spend)
    VALUES ('plain', 7.0);
INSERT INTO "LiteLLM_EndUserTable" (user_id, object_permission_id)
    VALUES ('permission-user', 'permission-1');
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["auth-end-users-postgres-tests"])
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
    (fixture, pool)
}

async fn seed(pool: &Pool) {
    let client = pool.get().await.expect("the database connection");
    client.batch_execute(SEED).await.expect("the seeded rows");
}

fn store(pool: &Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

fn database_rows(pool: &Pool) -> DatabaseEndUsers {
    DatabaseEndUsers {
        writer: Arc::new(store(pool)),
        reader: Arc::new(store(pool)),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    }
}

fn guard_off() -> EndUserRegistry {
    EndUserRegistry::new(Settings {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: None,
    })
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

fn request(end_user_id: &str) -> EndUserRequest<'_> {
    EndUserRequest {
        end_user_id: Some(end_user_id),
        key_end_user_budget_id: None,
        token_end_user_max_budget: None,
    }
}

const fn end_user_objects<'deps>(
    rows: &'deps DatabaseEndUsers,
    store: &'deps PostgresStore,
    cache: &'deps TestCache,
    registry: &'deps EndUserRegistry,
    max_end_user_budget_id: Option<&'deps str>,
) -> EndUserObjects<'deps, DatabaseEndUsers, DatabaseEndUsers, TestCache> {
    EndUserObjects {
        rows: Some(rows),
        budgets: Some(rows),
        cache,
        registry,
        store,
        max_end_user_budget_id,
    }
}

async fn resolved(
    objects: &EndUserObjects<'_, DatabaseEndUsers, DatabaseEndUsers, TestCache>,
    end_user_id: &str,
) -> LiteLLMEndUserTable {
    objects
        .get_end_user_object(&request(end_user_id))
        .await
        .expect("the row resolves")
        .expect("the row exists")
}

fn relation_budget_id(end_user: &LiteLLMEndUserTable) -> Option<&str> {
    end_user.litellm_budget_table.as_ref()?.budget_id.as_deref()
}

// the budget decision over a live database spend, with an optional conflicting cached counter
async fn check_with_cached_spend(
    store: &PostgresStore,
    end_user: &LiteLLMEndUserTable,
    cached: Option<Decimal>,
) -> Result<(), Decimal> {
    let cache = spend_dual_cache();
    if let Some(amount) = cached {
        cache
            .memory_set(&format!("spend:end_user:{}", end_user.user_id), amount)
            .await
            .expect("the conflicting cached spend");
    }
    let reseed = reseed_over(Some(SqlSpendCounterRows::from(store.clone())));
    let current = CurrentSpend {
        cache: &cache,
        batch: &NoSpendBatch,
        reseed: &reseed,
        fail_closed_budget_enforcement: false,
    };
    match check_end_user_budget(end_user, "/v1/chat/completions", &current).await {
        Ok(()) => Ok(()),
        Err(EndUserBudgetError::Exceeded(exceeded)) => Err(exceeded.details.current_cost),
        Err(EndUserBudgetError::Spend(source)) => panic!("the spend read failed: {source}"),
    }
}

async fn stored_budget_id(pool: &Pool, user_id: &str) -> Option<String> {
    let client = pool.get().await.expect("the database connection");
    let row = client
        .query_one(
            r#"SELECT "budget_id" FROM "LiteLLM_EndUserTable" WHERE "user_id" = $1"#,
            &[&user_id],
        )
        .await
        .expect("the stored end-user row");
    row.get(0)
}

#[tokio::test]
async fn relation_includes() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    let rows = database_rows(&pool);
    let store = store(&pool);
    let cache = test_cache();
    let registry = guard_off();
    let objects = end_user_objects(&rows, &store, &cache, &registry, None);
    let with_budget = resolved(&objects, "with-budget").await;
    assert_eq!(with_budget.budget_id.as_deref(), Some("budget-1"));
    assert_eq!(with_budget.spend, money("5"));
    let relation = with_budget
        .litellm_budget_table
        .as_ref()
        .expect("the budget relation is included");
    assert_eq!(relation.budget_id.as_deref(), Some("budget-1"));
    assert_eq!(relation.max_budget, Some(money("100")));
    assert!(with_budget.object_permission.is_none());
    let permission_user = resolved(&objects, "permission-user").await;
    let permission = permission_user
        .object_permission
        .as_ref()
        .expect("the object permission relation is included");
    assert_eq!(permission.object_permission_id, "permission-1");
    assert!(permission_user.litellm_budget_table.is_none());
    close(fixture, pool).await;
}

#[tokio::test]
async fn database_rows_and_conflicting_spend() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    let rows = database_rows(&pool);
    let store = store(&pool);
    let cache = test_cache();
    let registry = guard_off();
    let objects = end_user_objects(&rows, &store, &cache, &registry, Some("budget-1"));
    let plain = resolved(&objects, "plain").await;
    assert!(plain.budget_id.is_none());
    assert_eq!(relation_budget_id(&plain), Some("budget-1"));
    assert!(stored_budget_id(&pool, "plain").await.is_none());
    let with_budget = resolved(&objects, "with-budget").await;
    let cost = check_with_cached_spend(&store, &with_budget, Some(money("150")))
        .await
        .expect_err("the cached spend exceeds the budget");
    assert_eq!(cost, money("150"));
    check_with_cached_spend(&store, &with_budget, None)
        .await
        .expect("the database spend of the row is within budget");
    close(fixture, pool).await;
}

// the configured lookup deadline bounds the row read, so a stalled database cannot park the request
#[tokio::test]
async fn end_user_read_carries_the_lookup_deadline() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    let holder = pool.get().await.expect("the lock-holding connection");
    holder
        .batch_execute(r#"BEGIN; LOCK TABLE "LiteLLM_EndUserTable" IN ACCESS EXCLUSIVE MODE"#)
        .await
        .expect("the lock on the end-user table");
    let mut rows = database_rows(&pool);
    rows.default_deadline_seconds = 0.2_f64;
    let error = rows
        .find_end_user("with-budget")
        .await
        .expect_err("a lookup blocked behind the table lock exceeds the deadline");
    let EndUserFetchError::Read(source) = error;
    assert!(matches!(
        *source,
        AuthReadError::Lookup(DbLookupError::Deadline(_))
    ));
    holder
        .batch_execute("ROLLBACK")
        .await
        .expect("the lock release");
    drop(holder);
    close(fixture, pool).await;
}
