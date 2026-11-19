use std::sync::{Arc, Mutex};

use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::table::ReadContext;
use crate::auth_resource_grants::postgres::open_grant_rows;
use crate::auth_tag_budgets::groups::model_access_group_budget;
use crate::auth_tag_budgets::rows::group::ModelAccessGroupBudgetTable;
use crate::auth_tag_budgets::rows::tag::TagBudgetTable;
use crate::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow};

const SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" ("budget_id", "max_budget", "created_by", "updated_by")
    VALUES ('budget-cap', 12.5, 'seed', 'seed'), ('budget-zero', 0, 'seed', 'seed');
INSERT INTO "LiteLLM_TagTable" ("tag_name", "models", "spend", "budget_id")
    VALUES ('tag-a', ARRAY['m1'], 1.5, 'budget-cap'),
           ('tag-b', ARRAY[]::text[], 0, NULL),
           ('tag-c', NULL, 2.25, 'budget-zero');
INSERT INTO "LiteLLM_ModelAccessGroupBudgetTable" ("access_group_name", "spend", "budget_id")
    VALUES ('group-a', 3.5, 'budget-cap'),
           ('group-b', 0, NULL);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["auth-tag-budgets-postgres-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the database fixture");
    let pool = Pool::builder(Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    ))
    .max_size(2)
    .build()
    .expect("the database pool");
    (fixture, pool)
}

async fn seed(pool: &Pool) {
    let client = pool.get().await.expect("the database connection");
    client.batch_execute(SEED).await.expect("the seeded rows");
}

async fn close(fixture: PostgresFixture, pool: &Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

fn context() -> ReadContext<SystemClock> {
    ReadContext {
        default_deadline_seconds: 5.0,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn tag_row<'rows>(rows: &'rows [TagTable], name: &str) -> &'rows TagTable {
    rows.iter()
        .find(|row| row.tag_name == name)
        .expect("the seeded tag row")
}

fn assert_tag_rows(rows: &[TagTable]) {
    assert_eq!(rows.len(), 3);
    let tag_a = tag_row(rows, "tag-a");
    assert_eq!(tag_a.models, ["m1"]);
    assert_eq!(tag_a.spend, Decimal::new(15, 1));
    let capped = tag_a
        .litellm_budget_table
        .as_ref()
        .expect("the joined budget row");
    assert_eq!(capped.budget_id.as_deref(), Some("budget-cap"));
    assert_eq!(capped.max_budget, Some(Decimal::new(125, 1)));
    let tag_b = tag_row(rows, "tag-b");
    assert!(tag_b.litellm_budget_table.is_none());
    assert!(tag_b.models.is_empty());
    let tag_c = tag_row(rows, "tag-c");
    assert!(tag_c.models.is_empty());
    let zero = tag_c
        .litellm_budget_table
        .as_ref()
        .expect("the zero budget row");
    assert_eq!(zero.max_budget, Some(Decimal::ZERO));
}

fn assert_group_rows(rows: &[ModelAccessGroupBudgetRow]) {
    assert_eq!(rows.len(), 2);
    let group_a = rows
        .iter()
        .find(|row| row.access_group_name == "group-a")
        .expect("the seeded group row");
    assert_eq!(group_a.spend, Some(Decimal::new(35, 1)));
    let projected = model_access_group_budget(group_a);
    assert_eq!(projected.max_budget, Some(Decimal::new(125, 1)));
    let group_b = rows
        .iter()
        .find(|row| row.access_group_name == "group-b")
        .expect("the seeded group row");
    assert_eq!(model_access_group_budget(group_b).max_budget, None);
}

async fn assert_row_counts(pool: &Pool) {
    let connection = pool.get().await.expect("the count connection");
    let tags: i64 = connection
        .query_one("SELECT count(*) FROM \"LiteLLM_TagTable\"", &[])
        .await
        .expect("the tag count")
        .get(0);
    let groups: i64 = connection
        .query_one(
            "SELECT count(*) FROM \"LiteLLM_ModelAccessGroupBudgetTable\"",
            &[],
        )
        .await
        .expect("the group count")
        .get(0);
    assert_eq!((tags, groups), (3, 2));
}

#[tokio::test]
async fn postgres_batch_relations() {
    let (fixture, pool) = database().await;
    seed(&pool).await;
    let client = Arc::new(pool.get().await.expect("the database connection"));
    let tags = open_grant_rows(
        Arc::clone(&client),
        TagBudgetTable,
        AuthReadAdapter::Tag,
        context(),
    );
    let found = tags
        .by_names(names(&["tag-a", "tag-b", "tag-c", "tag-missing"]))
        .await
        .expect("the tag batch read");
    assert_tag_rows(&found);
    let registry = tags
        .registry_rows(5001)
        .await
        .expect("the tag registry rows");
    assert_eq!(registry.len(), 3);
    let groups = open_grant_rows(
        client,
        ModelAccessGroupBudgetTable,
        AuthReadAdapter::ModelAccessGroupBudget,
        context(),
    );
    let budgets = groups
        .by_names(names(&["group-a", "group-b", "group-missing"]))
        .await
        .expect("the group batch read");
    assert_group_rows(&budgets);
    let group_registry = groups
        .registry_rows(5001)
        .await
        .expect("the group registry rows");
    assert_eq!(group_registry.len(), 2);
    drop(tags);
    drop(groups);
    assert_row_counts(&pool).await;
    close(fixture, &pool).await;
}
