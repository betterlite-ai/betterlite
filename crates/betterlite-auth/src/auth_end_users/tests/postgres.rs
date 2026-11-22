#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_config::sections::end_user_registry::Settings;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::SecretString;
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use super::super::check::{EndUserBudgetError, check_end_user_budget};
use super::super::object::{EndUserObjects, EndUserRequest};
use super::super::postgres::DatabaseEndUsers;
use super::support::{TestCache, money, reseed_over, spend_dual_cache, test_cache};
use crate::current_spend::CurrentSpend;
use crate::end_user_registry::EndUserRegistry;
use crate::spend_reads::batch::NoSpendBatch;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::sql::SqlSpendCounterRows;

const SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, created_by, updated_by)
    VALUES ('budget-1', 100.0, 'tester', 'tester');
INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id)
    VALUES ('permission-1');
INSERT INTO "LiteLLM_EndUserTable" (user_id, spend, budget_id)
    VALUES ('with-budget', 5.0, 'budget-1');
INSERT INTO "LiteLLM_EndUserTable" (user_id, budget_id)
    VALUES ('dangling', 'missing');
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
    let cache: TestCache = test_cache();
    let registry = guard_off();
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&rows),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: None,
    };
    let with_budget = objects
        .get_end_user_object(&EndUserRequest {
            end_user_id: Some("with-budget"),
            key_end_user_budget_id: None,
            token_end_user_max_budget: None,
        })
        .await
        .expect("the row resolves")
        .expect("the row exists");
    assert_eq!(with_budget.budget_id.as_deref(), Some("budget-1"));
    assert_eq!(with_budget.spend, money("5"));
    let relation = with_budget
        .litellm_budget_table
        .as_ref()
        .expect("the budget relation is included");
    assert_eq!(relation.budget_id.as_deref(), Some("budget-1"));
    assert_eq!(relation.max_budget, Some(money("100")));
    assert!(with_budget.object_permission.is_none());

    let permission_user = objects
        .get_end_user_object(&EndUserRequest {
            end_user_id: Some("permission-user"),
            key_end_user_budget_id: None,
            token_end_user_max_budget: None,
        })
        .await
        .expect("the row resolves")
        .expect("the row exists");
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
    let objects = EndUserObjects {
        rows: Some(&rows),
        budgets: Some(&rows),
        cache: &cache,
        registry: &registry,
        store: &store,
        max_end_user_budget_id: Some("budget-1"),
    };
    let request = |end_user_id: &str| EndUserRequest {
        end_user_id: Some(end_user_id),
        key_end_user_budget_id: None,
        token_end_user_max_budget: None,
    };
    let plain = objects
        .get_end_user_object(&request("plain"))
        .await
        .expect("the plain row resolves")
        .expect("the row exists");
    assert!(plain.budget_id.is_none());
    assert_eq!(
        plain
            .litellm_budget_table
            .as_ref()
            .and_then(|budget| budget.budget_id.as_deref()),
        Some("budget-1")
    );
    let dangling = objects
        .get_end_user_object(&request("dangling"))
        .await
        .expect("the dangling row resolves")
        .expect("the row exists");
    assert_eq!(dangling.budget_id.as_deref(), Some("missing"));
    assert_eq!(
        dangling
            .litellm_budget_table
            .as_ref()
            .and_then(|budget| budget.budget_id.as_deref()),
        Some("budget-1")
    );
    assert_eq!(
        stored_budget_id(&pool, "dangling").await.as_deref(),
        Some("missing")
    );
    assert!(stored_budget_id(&pool, "plain").await.is_none());

    let with_budget = objects
        .get_end_user_object(&request("with-budget"))
        .await
        .expect("the budget row resolves")
        .expect("the row exists");
    let cached_spend = spend_dual_cache();
    cached_spend
        .memory_set("spend:end_user:with-budget", money("150"))
        .await
        .expect("the conflicting cached spend");
    let cached_reseed = reseed_over(Some(SqlSpendCounterRows::from(store.clone())));
    let cached = CurrentSpend {
        cache: &cached_spend,
        batch: &NoSpendBatch,
        reseed: &cached_reseed,
        fail_closed_budget_enforcement: false,
    };
    let error = check_end_user_budget(&with_budget, "/v1/chat/completions", &cached)
        .await
        .expect_err("the cached spend exceeds the budget");
    let EndUserBudgetError::Exceeded(exceeded) = error else {
        panic!("an overrun is a budget rejection");
    };
    assert_eq!(exceeded.details.current_cost, money("150"));

    let cold_spend = spend_dual_cache();
    let cold_reseed = reseed_over(Some(SqlSpendCounterRows::from(store.clone())));
    let cold = CurrentSpend {
        cache: &cold_spend,
        batch: &NoSpendBatch,
        reseed: &cold_reseed,
        fail_closed_budget_enforcement: false,
    };
    check_end_user_budget(&with_budget, "/v1/chat/completions", &cold)
        .await
        .expect("the database spend of the row is within budget");
    close(fixture, pool).await;
}
