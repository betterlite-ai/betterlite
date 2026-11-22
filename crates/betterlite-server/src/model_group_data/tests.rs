#![cfg(test)]

mod budgets;
mod deployments;
mod discovery;
mod fixtures;
mod ports;
mod reload;

use std::collections::{BTreeMap, BTreeSet};

use axum::http::StatusCode;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use tokio_postgres::NoTls;

use self::fixtures::json_object;
use self::fixtures::scripted::ScriptedDatabase;
use super::deployments::{DeploymentSelection, tag_deployments};
use super::errors::{GroupDataError, HttpFailure};
use super::ports::{BudgetInclude, GroupBudgetCreate, GroupBudgetUpdate, GroupDatabase};
use super::store::PostgresGroupData;
use super::types::{BudgetRow, DeploymentRow, GroupBudgetRow};

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite number"))
}

fn list(values: &[&str]) -> Json {
    Json::List(values.iter().map(|value| text(value)).collect())
}

fn deployment(model_id: &str, model_name: &str, model_info: Option<Json>) -> DeploymentRow {
    DeploymentRow {
        model_id: model_id.to_owned(),
        model_name: model_name.to_owned(),
        model_info,
    }
}

fn tagged_deployment(model_id: &str, model_name: &str, access_groups: &[&str]) -> DeploymentRow {
    deployment(
        model_id,
        model_name,
        Some(Json::Object(json_object(&[(
            "access_groups",
            list(access_groups),
        )]))),
    )
}

fn capped_budget() -> BudgetRow {
    BudgetRow {
        budget_id: "budget-1".to_owned(),
        max_budget: Some(Decimal::new(250, 1)),
        soft_budget: Some(Decimal::new(200, 1)),
        budget_duration: Some("30d".to_owned()),
        budget_reset_at: Some(Timestamp::Naive(
            "2026-01-01T00:00:00".parse().expect("a naive timestamp"),
        )),
    }
}

fn linked_group(
    access_group: &str,
    spend: Option<Decimal>,
    budget_id: Option<&str>,
    budget: Option<BudgetRow>,
) -> GroupBudgetRow {
    GroupBudgetRow {
        access_group_name: access_group.to_owned(),
        spend,
        budget_id: budget_id.map(str::to_owned),
        litellm_budget_table: budget,
    }
}

fn group_database(deployments: Vec<DeploymentRow>) -> ScriptedDatabase {
    ScriptedDatabase::new(deployments, Vec::new(), Vec::new())
}

fn http_failure(error: GroupDataError) -> HttpFailure {
    match error {
        GroupDataError::Http(failure) => failure,
        other => panic!("expected an HTTP failure, got {other:?}"),
    }
}

fn value_error_message(error: GroupDataError) -> String {
    match error {
        GroupDataError::ValueError { message } => message,
        other => panic!("expected a value error, got {other:?}"),
    }
}

fn runtime_message(error: GroupDataError) -> String {
    match error {
        GroupDataError::Runtime { message } => message,
        other => panic!("expected a runtime error, got {other:?}"),
    }
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn postgres_pool() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["model-group-data-postgres-tests"])
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
    .max_size(1)
    .build()
    .expect("the database pool");
    (fixture, pool)
}

struct StoreFixture {
    store: PostgresStore,
    availability: Availability,
}

impl StoreFixture {
    fn new(pool: &Pool) -> Self {
        Self {
            store: PostgresStore {
                pool: Some(pool.clone()),
            },
            availability: Availability::default(),
        }
    }

    fn database(&self) -> PostgresGroupData<'_> {
        PostgresGroupData {
            client: Some(PrismaClient {
                writer: &self.store,
                reader: &self.store,
                availability: &self.availability,
            }),
        }
    }
}

async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.expect("the database connection");
    client.batch_execute(sql).await.expect("the seeded rows");
}

async fn scalar_text(pool: &Pool, sql: &str, parameter: &str) -> String {
    let client = pool.get().await.expect("the database connection");
    let row = client
        .query_one(sql, &[&parameter])
        .await
        .expect("the selected row");
    row.get(0)
}

const SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable"
    ("budget_id", "max_budget", "soft_budget", "budget_duration", "budget_reset_at", "created_by", "updated_by")
    VALUES ('budget-1', 25.0, 20.0, '30d', '2026-01-01 00:00:00', 'seed', 'seed'),
           ('budget-2', NULL, NULL, NULL, NULL, 'seed', 'seed');
INSERT INTO "LiteLLM_ProxyModelTable"
    ("model_id", "model_name", "litellm_params", "model_info", "created_by", "updated_by")
    VALUES ('d1', 'shared', '{"api_key": "sk-1"}', '{"ciphertext": "secret"}', 'seed', 'seed'),
           ('d2', 'shared', '{}', NULL, 'seed', 'seed'),
           ('d3', 'other', '{}', '"{\"nested\": true}"', 'seed', 'seed'),
           ('d4', 'null-info', '{}', 'null', 'seed', 'seed');
INSERT INTO "LiteLLM_ModelAccessGroupBudgetTable"
    ("access_group_name", "spend", "budget_id", "created_by", "updated_by")
    VALUES ('group-a', 3.5, 'budget-1', 'seed', 'seed'),
           ('group-b', 0, NULL, 'seed', 'seed');
"#;

#[tokio::test]
async fn postgres_pinned_rows() {
    let (fixture, pool) = postgres_pool().await;
    execute(&pool, SEED).await;
    let fixture_store = StoreFixture::new(&pool);
    let database = fixture_store.database();
    let deployments = database
        .deployment_find_many(None)
        .await
        .expect("every deployment");
    assert_eq!(deployments.len(), 4);
    let shared = database
        .deployment_find_many(Some(&names(&["shared"])))
        .await
        .expect("the shared deployments");
    let shared_ids: BTreeSet<String> = shared.into_iter().map(|row| row.model_id).collect();
    assert_eq!(shared_ids, ids(&["d1", "d2"]));
    let d3 = database
        .deployment_find_unique("d3")
        .await
        .expect("the stored row")
        .expect("the seeded deployment");
    assert_eq!(
        d3.model_info,
        Some(Json::Text(r#"{"nested": true}"#.to_owned()))
    );
    let d4 = database
        .deployment_find_unique("d4")
        .await
        .expect("the stored row")
        .expect("the seeded deployment");
    assert_eq!(d4.model_info, None);
    assert!(
        database
            .deployment_find_unique("missing")
            .await
            .expect("the stored row")
            .is_none()
    );
    database
        .update_model_info(
            "d1",
            r#"{"ciphertext": "secret", "access_groups": ["group-a"]}"#,
        )
        .await
        .expect("the write");
    let updated = database
        .deployment_find_unique("d1")
        .await
        .expect("the stored row")
        .expect("the seeded deployment");
    assert_eq!(
        updated.model_info,
        Some(Json::Object(json_object(&[
            ("ciphertext", text("secret")),
            ("access_groups", list(&["group-a"])),
        ])))
    );
    assert_eq!(
        scalar_text(
            &pool,
            r#"SELECT litellm_params::TEXT FROM "LiteLLM_ProxyModelTable" WHERE model_id = $1"#,
            "d1",
        )
        .await,
        r#"{"api_key": "sk-1"}"#
    );
    let include = BudgetInclude {
        litellm_budget_table: true,
    };
    let joined = database
        .budget_find_unique("group-a", include)
        .await
        .expect("the budget row")
        .expect("the seeded group");
    assert_eq!(joined.spend, Some(Decimal::new(35, 1)));
    let budget = joined.litellm_budget_table.expect("the joined budget");
    assert_eq!(budget.budget_id, "budget-1");
    assert_eq!(budget.max_budget, Some(Decimal::new(250, 1)));
    assert_eq!(budget.soft_budget, Some(Decimal::new(200, 1)));
    assert_eq!(budget.budget_duration.as_deref(), Some("30d"));
    assert_eq!(
        budget.budget_reset_at,
        Some(Timestamp::Naive(
            "2026-01-01T00:00:00".parse().expect("a naive timestamp"),
        ))
    );
    let plain = database
        .budget_find_unique(
            "group-a",
            BudgetInclude {
                litellm_budget_table: false,
            },
        )
        .await
        .expect("the budget row")
        .expect("the seeded group");
    assert_eq!(plain.litellm_budget_table, None);
    let every = database
        .budget_find_many(include)
        .await
        .expect("every budget row");
    assert_eq!(every.len(), 2);
    let created = database
        .budget_upsert(
            &GroupBudgetCreate {
                access_group_name: "group-c".to_owned(),
                budget_id: Some("budget-2".to_owned()),
                created_by: "creator".to_owned(),
                updated_by: "creator".to_owned(),
            },
            &GroupBudgetUpdate {
                budget_id: Some("budget-2".to_owned()),
                updated_by: "creator".to_owned(),
            },
            include,
        )
        .await
        .expect("the created row");
    assert_eq!(created.spend, Some(Decimal::ZERO));
    assert_eq!(
        created
            .litellm_budget_table
            .expect("the joined budget")
            .max_budget,
        None
    );
    execute(
        &pool,
        r#"UPDATE "LiteLLM_ModelAccessGroupBudgetTable" SET spend = 7 WHERE access_group_name = 'group-c'"#,
    )
    .await;
    let rewritten = database
        .budget_upsert(
            &GroupBudgetCreate {
                access_group_name: "group-c".to_owned(),
                budget_id: Some("budget-1".to_owned()),
                created_by: "creator".to_owned(),
                updated_by: "creator".to_owned(),
            },
            &GroupBudgetUpdate {
                budget_id: Some("budget-1".to_owned()),
                updated_by: "editor".to_owned(),
            },
            include,
        )
        .await
        .expect("the updated row");
    assert_eq!(rewritten.spend, Some(Decimal::new(70, 1)));
    assert_eq!(
        rewritten
            .litellm_budget_table
            .expect("the joined budget")
            .budget_id,
        "budget-1"
    );
    let actors: BTreeMap<String, String> = {
        let client = pool.get().await.expect("the database connection");
        let row = client
            .query_one(
                r#"SELECT created_by, updated_by FROM "LiteLLM_ModelAccessGroupBudgetTable" WHERE access_group_name = 'group-c'"#,
                &[],
            )
            .await
            .expect("the stored actors");
        BTreeMap::from([
            ("created_by".to_owned(), row.get(0)),
            ("updated_by".to_owned(), row.get(1)),
        ])
    };
    assert_eq!(actors["created_by"], "creator");
    assert_eq!(actors["updated_by"], "editor");
    let deleted = database
        .budget_delete("group-a")
        .await
        .expect("the delete")
        .expect("the seeded group");
    assert_eq!(deleted.spend, Some(Decimal::new(35, 1)));
    assert_eq!(deleted.litellm_budget_table, None);
    assert!(
        database
            .budget_delete("group-a")
            .await
            .expect("the second delete")
            .is_none()
    );
    assert_eq!(
        scalar_text(
            &pool,
            r#"SELECT count(*)::TEXT FROM "LiteLLM_BudgetTable" WHERE budget_id = $1"#,
            "budget-1",
        )
        .await,
        "1"
    );
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

const FAILURE_SEED: &str = r#"
INSERT INTO "LiteLLM_ProxyModelTable"
    ("model_id", "model_name", "litellm_params", "created_by", "updated_by")
    VALUES ('f1', 'alpha', '{}', 'seed', 'seed'),
           ('f2', 'beta', '{}', 'seed', 'seed');
"#;

#[tokio::test]
async fn postgres_failure_effects() {
    let (fixture, pool) = postgres_pool().await;
    execute(&pool, FAILURE_SEED).await;
    let fixture_store = StoreFixture::new(&pool);
    let database = fixture_store.database();
    let model_names = names(&["alpha", "missing"]);
    let error = tag_deployments(
        DeploymentSelection::ByNames(&model_names),
        "group-a",
        &database,
    )
    .await
    .expect_err("the missing name");
    assert_eq!(
        http_failure(error).message,
        "Can't find model 'missing' in Database. Access group management is only supported for database models."
    );
    assert_eq!(
        scalar_text(
            &pool,
            r#"SELECT COALESCE(model_info::TEXT, '') FROM "LiteLLM_ProxyModelTable" WHERE model_id = $1"#,
            "f1",
        )
        .await,
        ""
    );
    let model_ids = names(&["f1", "missing", "f2"]);
    let error = tag_deployments(DeploymentSelection::ByIds(&model_ids), "group-a", &database)
        .await
        .expect_err("the missing id");
    assert_eq!(
        http_failure(error).message,
        "Deployment with model_id 'missing' not found in Database."
    );
    assert_eq!(
        scalar_text(
            &pool,
            r#"SELECT model_info::TEXT FROM "LiteLLM_ProxyModelTable" WHERE model_id = $1"#,
            "f1",
        )
        .await,
        r#"{"access_groups": ["group-a"]}"#
    );
    assert_eq!(
        scalar_text(
            &pool,
            r#"SELECT COALESCE(model_info::TEXT, '') FROM "LiteLLM_ProxyModelTable" WHERE model_id = $1"#,
            "f2",
        )
        .await,
        ""
    );
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}

#[tokio::test]
async fn postgres_missing_client() {
    let database: PostgresGroupData<'_> = PostgresGroupData { client: None };
    let error = database
        .deployment_find_many(None)
        .await
        .expect_err("the missing database");
    let failure = http_failure(error);
    assert_eq!(failure.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(failure.message, "Database not connected.");
    let error = database
        .budget_delete("group-a")
        .await
        .expect_err("the missing database");
    assert_eq!(http_failure(error).message, "Database not connected.");
}
