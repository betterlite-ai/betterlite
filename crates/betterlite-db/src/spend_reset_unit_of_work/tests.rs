#![cfg(test)]

use super::batch::{BudgetCascadeBatch, CommitError, SpendResetBatch};
use super::queue::{BudgetIdFilter, KeyLinkedResetFilter, LinkedResetFilter, SpendRange};
use super::{
    BudgetCascadeUnitOfWork, SpendDecrement, SpendResetUnitOfWork, UnitOfWorkError,
    budget_cascade_unit_of_work, spend_reset_data, spend_reset_unit_of_work,
};
use crate::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use std::future::{Future as _, pending, poll_fn};
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_postgres::NoTls;

#[derive(Debug, thiserror::Error)]
#[error("synthetic body error")]
struct TestError;

#[test]
fn spend_reset_only_two_fields() {
    let data = spend_reset_data(None, Decimal::from(5_i32));
    let json = serde_json::to_value(data).unwrap();
    let object = json.as_object().unwrap();
    assert_eq!(object.len(), 2);
    assert!(object.contains_key("spend"));
    assert!(object.contains_key("budget_reset_at"));
}

#[test]
fn null_reset_time_is_explicit() {
    let data = spend_reset_data(None, Decimal::from(5_i32));
    let json = serde_json::to_value(data).unwrap();
    assert!(json.as_object().unwrap().contains_key("budget_reset_at"));
    assert_eq!(json["budget_reset_at"], serde_json::Value::Null);
}

#[test]
fn zero_decrement_retains_window_write() {
    let at = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
    let data = spend_reset_data(Some(at), Decimal::ZERO);
    assert_eq!(
        data.spend,
        SpendDecrement {
            decrement: Decimal::ZERO
        }
    );
    assert_eq!(data.budget_reset_at, Some(at));
}

#[test]
fn negative_decrement_not_clamped() {
    let data = spend_reset_data(None, Decimal::from(-5_i32));
    assert_eq!(data.spend.decrement, Decimal::from(-5_i32));
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, PostgresStore) {
    let url = Environment::parse_from(["spend-reset-unit-of-work-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let pool = pool(&fixture.options.url);
    (fixture, PostgresStore { pool: Some(pool) })
}

fn pool(url: &SecretString) -> Pool {
    let config = url.expose_secret().parse().unwrap();
    let manager = Manager::new(config, NoTls);
    Pool::builder(manager).max_size(2).build().unwrap()
}

async fn finish(fixture: PostgresFixture, store: PostgresStore, client: Client) {
    drop(client);
    store.pool.unwrap().close();
    fixture.close_async().await.unwrap();
}

async fn schema() -> (PostgresFixture, PostgresStore, Client) {
    let (fixture, store) = database().await;
    let client = store.session().await.unwrap();
    client
        .batch_execute(
            "CREATE TABLE \"LiteLLM_VerificationToken\" (token TEXT PRIMARY KEY, spend DOUBLE PRECISION NOT NULL DEFAULT 0, budget_reset_at TIMESTAMP(3), budget_id TEXT, budget_duration TEXT, label TEXT NOT NULL DEFAULT 'kept'); CREATE TABLE \"LiteLLM_UserTable\" (user_id TEXT PRIMARY KEY, spend DOUBLE PRECISION NOT NULL DEFAULT 0, budget_reset_at TIMESTAMP(3), total_spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_TeamTable\" (team_id TEXT PRIMARY KEY, spend DOUBLE PRECISION NOT NULL DEFAULT 0, budget_reset_at TIMESTAMP(3)); CREATE TABLE \"LiteLLM_TeamMembership\" (user_id TEXT, team_id TEXT, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0, PRIMARY KEY (user_id, team_id)); CREATE TABLE \"LiteLLM_OrganizationTable\" (organization_id TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_TagTable\" (tag_name TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_ModelAccessGroupBudgetTable\" (access_group_name TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_ProjectTable\" (project_id TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_EndUserTable\" (user_id TEXT PRIMARY KEY, budget_id TEXT, spend DOUBLE PRECISION NOT NULL DEFAULT 0); CREATE TABLE \"LiteLLM_BudgetTable\" (budget_id TEXT PRIMARY KEY, budget_reset_at TIMESTAMP(3));",
        )
        .await
        .unwrap();
    (fixture, store, client)
}

async fn spend_of(client: &Client, table: &str, column: &str, id: &str) -> f64 {
    let sql = format!("SELECT spend FROM \"{table}\" WHERE {column} = $1");
    client.query_one(sql.as_str(), &[&id]).await.unwrap().get(0)
}

async fn assert_spend(client: &Client, table: &str, column: &str, id: &str, expected: f64) {
    let actual = spend_of(client, table, column, id).await;
    assert_eq!(actual.to_bits(), expected.to_bits());
}

async fn reset_at_of(
    client: &Client,
    table: &str,
    column: &str,
    id: &str,
) -> Option<NaiveDateTime> {
    let sql = format!("SELECT budget_reset_at FROM \"{table}\" WHERE {column} = $1");
    client.query_one(sql.as_str(), &[&id]).await.unwrap().get(0)
}

async fn seed_bindings(client: &Client) {
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend) VALUES ('token-a', 10); INSERT INTO \"LiteLLM_UserTable\" (user_id, spend) VALUES ('user-a', 10); INSERT INTO \"LiteLLM_TeamTable\" (team_id, spend) VALUES ('team-a', 10); INSERT INTO \"LiteLLM_TeamMembership\" (user_id, team_id, budget_id, spend) VALUES ('u', 't', 'budget-a', 10); INSERT INTO \"LiteLLM_VerificationToken\" (token, budget_id, spend) VALUES ('token-linked', 'budget-a', 10), ('token-window', 'budget-a', 10); UPDATE \"LiteLLM_VerificationToken\" SET budget_duration = 'daily' WHERE token = 'token-window'; INSERT INTO \"LiteLLM_OrganizationTable\" (organization_id, budget_id, spend) VALUES ('org-a', 'budget-a', 10); INSERT INTO \"LiteLLM_TagTable\" (tag_name, budget_id, spend) VALUES ('tag-a', 'budget-a', 10); INSERT INTO \"LiteLLM_ModelAccessGroupBudgetTable\" (access_group_name, budget_id, spend) VALUES ('group-a', 'budget-a', 10); INSERT INTO \"LiteLLM_ProjectTable\" (project_id, budget_id, spend) VALUES ('project-a', 'budget-a', 10); INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('enduser-a', 'budget-a', 10); INSERT INTO \"LiteLLM_BudgetTable\" (budget_id) VALUES ('budget-a');",
        )
        .await
        .unwrap();
}

async fn queue_resets(store: &PostgresStore, at: DateTime<Utc>) {
    spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.keys
                .queue_spend_reset("token-a", Some(at), Decimal::from(3_i32));
            unit.users
                .queue_spend_reset("user-a", Some(at), Decimal::from(3_i32));
            unit.teams
                .queue_spend_reset("team-a", Some(at), Decimal::from(3_i32));
            Ok(())
        },
    )
    .await
    .unwrap();
    budget_cascade_unit_of_work(
        || BudgetCascadeBatch {
            store: store.clone(),
        },
        async move |unit: &mut BudgetCascadeUnitOfWork| -> Result<(), TestError> {
            queue_cascade_writes(unit, at);
            Ok(())
        },
    )
    .await
    .unwrap();
}

fn queue_cascade_writes(unit: &BudgetCascadeUnitOfWork, at: DateTime<Utc>) {
    let filter = LinkedResetFilter {
        budget_id: BudgetIdFilter::Equals("budget-a".to_owned()),
        spend: SpendRange::default(),
    };
    let key_filter = KeyLinkedResetFilter {
        budget_id: BudgetIdFilter::Equals("budget-a".to_owned()),
        spend: SpendRange::default(),
    };
    unit.team_memberships
        .queue_spend_decrement(filter.clone(), Decimal::from(1_i32));
    unit.keys
        .queue_spend_decrement(key_filter, Decimal::from(1_i32));
    unit.organizations
        .queue_spend_decrement(filter.clone(), Decimal::from(2_i32));
    unit.tags
        .queue_spend_decrement(filter.clone(), Decimal::from(3_i32));
    unit.model_access_groups
        .queue_spend_decrement(filter.clone(), Decimal::from(4_i32));
    unit.projects
        .queue_spend_decrement(filter.clone(), Decimal::from(5_i32));
    unit.endusers
        .queue_spend_decrement(filter, Decimal::from(6_i32));
    unit.budgets.queue_window_advance("budget-a", at);
}

async fn assert_all_bindings(client: &Client, at: DateTime<Utc>) {
    for (table, column, id) in [
        ("LiteLLM_VerificationToken", "token", "token-a"),
        ("LiteLLM_UserTable", "user_id", "user-a"),
        ("LiteLLM_TeamTable", "team_id", "team-a"),
    ] {
        assert_spend(client, table, column, id, 7.0_f64).await;
    }
    assert_cascade_spends(client).await;
    assert_eq!(
        reset_at_of(client, "LiteLLM_BudgetTable", "budget_id", "budget-a").await,
        Some(at.naive_utc())
    );
}

async fn assert_cascade_spends(client: &Client) {
    for (table, column, id, expected) in [
        ("LiteLLM_TeamMembership", "team_id", "t", 9.0_f64),
        (
            "LiteLLM_VerificationToken",
            "token",
            "token-linked",
            9.0_f64,
        ),
        (
            "LiteLLM_OrganizationTable",
            "organization_id",
            "org-a",
            8.0_f64,
        ),
        ("LiteLLM_TagTable", "tag_name", "tag-a", 7.0_f64),
        (
            "LiteLLM_ModelAccessGroupBudgetTable",
            "access_group_name",
            "group-a",
            6.0_f64,
        ),
        ("LiteLLM_ProjectTable", "project_id", "project-a", 5.0_f64),
        ("LiteLLM_EndUserTable", "user_id", "enduser-a", 4.0_f64),
        (
            "LiteLLM_VerificationToken",
            "token",
            "token-window",
            10.0_f64,
        ),
    ] {
        assert_spend(client, table, column, id, expected).await;
    }
}

#[tokio::test]
async fn postgres_all_unit_of_work_table_bindings() {
    let (fixture, store, client) = schema().await;
    seed_bindings(&client).await;
    let at = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
    queue_resets(&store, at).await;
    assert_all_bindings(&client, at).await;
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn queued_body_sends_no_sql() {
    let (fixture, store, client) = schema().await;
    client
        .execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend) VALUES ('user-a', 10)",
            &[],
        )
        .await
        .unwrap();
    let probe = store.clone();
    spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.users
                .queue_spend_reset("user-a", None, Decimal::from(3_i32));
            let reader = probe.session().await.unwrap();
            assert_spend(&reader, "LiteLLM_UserTable", "user_id", "user-a", 10.0_f64).await;
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_spend(&client, "LiteLLM_UserTable", "user_id", "user-a", 7.0_f64).await;
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn clean_exit_commits_once() {
    let (fixture, store, client) = schema().await;
    client
        .execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend) VALUES ('user-a', 10)",
            &[],
        )
        .await
        .unwrap();
    spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.users
                .queue_spend_reset("user-a", None, Decimal::from(3_i32));
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_spend(&client, "LiteLLM_UserTable", "user_id", "user-a", 7.0_f64).await;
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn empty_batch_commits_once() {
    let (fixture, store, client) = schema().await;
    spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |_unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> { Ok(()) },
    )
    .await
    .unwrap();
    budget_cascade_unit_of_work(
        || BudgetCascadeBatch {
            store: store.clone(),
        },
        async move |_unit: &mut BudgetCascadeUnitOfWork| -> Result<(), TestError> { Ok(()) },
    )
    .await
    .unwrap();
    finish(fixture, store, client).await;
}

async fn cancel_body(store: &PostgresStore) {
    let (sender, mut receiver) = oneshot::channel();
    let mut operation = Box::pin(spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.keys
                .queue_spend_reset("token-a", None, Decimal::from(3_i32));
            sender.send(()).unwrap();
            pending::<()>().await;
            Ok(())
        },
    ));
    timeout(
        Duration::from_secs(5),
        poll_fn(|context| {
            assert!(operation.as_mut().poll(context).is_pending());
            Pin::new(&mut receiver).poll(context)
        }),
    )
    .await
    .unwrap()
    .unwrap();
    drop(operation);
}

#[tokio::test]
async fn body_error_and_cancel_discard_queue() {
    let (fixture, store, client) = schema().await;
    client
        .execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend) VALUES ('token-a', 10)",
            &[],
        )
        .await
        .unwrap();
    let result = spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.keys
                .queue_spend_reset("token-a", None, Decimal::from(3_i32));
            Err(TestError)
        },
    )
    .await;
    assert!(matches!(result, Err(UnitOfWorkError::Body(TestError))));
    cancel_body(&store).await;
    assert_spend(
        &client,
        "LiteLLM_VerificationToken",
        "token",
        "token-a",
        10.0_f64,
    )
    .await;
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_middle_statement_rolls_back_all() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute("INSERT INTO \"LiteLLM_VerificationToken\" (token, spend) VALUES ('token-a', 10); INSERT INTO \"LiteLLM_TeamTable\" (team_id, spend) VALUES ('team-a', 10);")
        .await
        .unwrap();
    let result = spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.keys
                .queue_spend_reset("token-a", None, Decimal::from(3_i32));
            unit.users
                .queue_spend_reset("missing-user", None, Decimal::from(3_i32));
            unit.teams
                .queue_spend_reset("team-a", None, Decimal::from(3_i32));
            Ok(())
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(UnitOfWorkError::Commit(CommitError::MissingResetTarget {
            table: "LiteLLM_UserTable"
        }))
    ));
    assert_spend(
        &client,
        "LiteLLM_VerificationToken",
        "token",
        "token-a",
        10.0_f64,
    )
    .await;
    assert_spend(&client, "LiteLLM_TeamTable", "team_id", "team-a", 10.0_f64).await;
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn rejected_commit_propagates_and_releases_connection() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute("INSERT INTO \"LiteLLM_UserTable\" (user_id, spend) VALUES ('dup-a', 10), ('dup-b', 8); ALTER TABLE \"LiteLLM_UserTable\" ADD CONSTRAINT dup_unique UNIQUE (spend) DEFERRABLE INITIALLY DEFERRED;")
        .await
        .unwrap();
    let result = spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.users
                .queue_spend_reset("dup-a", None, Decimal::from(2_i32));
            unit.users
                .queue_spend_reset("dup-b", None, Decimal::from(0_i32));
            Ok(())
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(UnitOfWorkError::Commit(CommitError::Rejected(_)))
    ));
    let probe = store.session().await.unwrap();
    assert_spend(&probe, "LiteLLM_UserTable", "user_id", "dup-a", 10.0_f64).await;
    assert_spend(&probe, "LiteLLM_UserTable", "user_id", "dup-b", 8.0_f64).await;
    drop(probe);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn commit_ack_loss_has_no_retry() {
    let (fixture, store, client) = schema().await;
    client
        .execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend) VALUES ('retry-user', 10)",
            &[],
        )
        .await
        .unwrap();
    client
        .batch_execute("CREATE SEQUENCE commit_seq; CREATE FUNCTION terminate_on_commit() RETURNS trigger AS $$ BEGIN PERFORM nextval('commit_seq'); PERFORM pg_terminate_backend(pg_backend_pid()); RETURN NULL; END; $$ LANGUAGE plpgsql; CREATE CONSTRAINT TRIGGER terminate_trigger AFTER UPDATE ON \"LiteLLM_UserTable\" DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (OLD.user_id = 'retry-user') EXECUTE FUNCTION terminate_on_commit();")
        .await
        .unwrap();
    let result = spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.users
                .queue_spend_reset("retry-user", None, Decimal::from(3_i32));
            Ok(())
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(UnitOfWorkError::Commit(CommitError::OutcomeUnknown(_)))
    ));
    let attempts: i64 = client
        .query_one("SELECT last_value FROM commit_seq", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(attempts, 1);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_decrement_keeps_concurrent_spend() {
    let (fixture, store, client) = schema().await;
    client
        .execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend) VALUES ('user-a', 10)",
            &[],
        )
        .await
        .unwrap();
    spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.users
                .queue_spend_reset("user-a", None, Decimal::from(3_i32));
            client
                .execute(
                    "UPDATE \"LiteLLM_UserTable\" SET spend = spend + 5 WHERE user_id = 'user-a'",
                    &[],
                )
                .await
                .unwrap();
            Ok(())
        },
    )
    .await
    .unwrap();
    let reader = store.session().await.unwrap();
    assert_spend(&reader, "LiteLLM_UserTable", "user_id", "user-a", 12.0_f64).await;
    finish(fixture, store, reader).await;
}

#[tokio::test]
async fn linked_filters_preserved_in_queue_order() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute("INSERT INTO \"LiteLLM_TagTable\" (tag_name, budget_id, spend) VALUES ('tag-a', 'budget-a', 10), ('tag-b', NULL, 10), ('tag-c', 'other-budget', 20);")
        .await
        .unwrap();
    budget_cascade_unit_of_work(
        || BudgetCascadeBatch {
            store: store.clone(),
        },
        async move |unit: &mut BudgetCascadeUnitOfWork| -> Result<(), TestError> {
            unit.tags.queue_spend_decrement(
                LinkedResetFilter {
                    budget_id: BudgetIdFilter::In(vec!["budget-a".to_owned()]),
                    spend: SpendRange {
                        greater_than: Some(Decimal::from(5_i32)),
                        less_than_or_equal: None,
                    },
                },
                Decimal::from(3_i32),
            );
            unit.tags.queue_spend_zero(LinkedResetFilter {
                budget_id: BudgetIdFilter::Equals("budget-a".to_owned()),
                spend: SpendRange {
                    greater_than: None,
                    less_than_or_equal: Some(Decimal::from(8_i32)),
                },
            });
            unit.tags.queue_spend_zero(LinkedResetFilter {
                budget_id: BudgetIdFilter::IsNull,
                spend: SpendRange::default(),
            });
            Ok(())
        },
    )
    .await
    .unwrap();
    // tag-a reads 0 only if the decrement (10 -> 7) ran before the <= 8 zero filter.
    assert_spend(&client, "LiteLLM_TagTable", "tag_name", "tag-a", 0.0_f64).await;
    assert_spend(&client, "LiteLLM_TagTable", "tag_name", "tag-b", 0.0_f64).await;
    assert_spend(&client, "LiteLLM_TagTable", "tag_name", "tag-c", 20.0_f64).await;
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn unmatched_linked_update_succeeds() {
    let (fixture, store, client) = schema().await;
    budget_cascade_unit_of_work(
        || BudgetCascadeBatch {
            store: store.clone(),
        },
        async move |unit: &mut BudgetCascadeUnitOfWork| -> Result<(), TestError> {
            unit.organizations.queue_spend_zero(LinkedResetFilter {
                budget_id: BudgetIdFilter::Equals("missing-budget".to_owned()),
                spend: SpendRange::default(),
            });
            Ok(())
        },
    )
    .await
    .unwrap();
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn deleted_budget_window_update_is_noop() {
    let (fixture, store, client) = schema().await;
    let at = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
    client
        .batch_execute("INSERT INTO \"LiteLLM_BudgetTable\" (budget_id) VALUES ('budget-del'); INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('enduser-del', 'budget-del', 10);")
        .await
        .unwrap();
    let deleter = store.session().await.unwrap();
    budget_cascade_unit_of_work(
        || BudgetCascadeBatch {
            store: store.clone(),
        },
        async move |unit: &mut BudgetCascadeUnitOfWork| -> Result<(), TestError> {
            unit.budgets.queue_window_advance("budget-del", at);
            unit.endusers.queue_spend_zero(LinkedResetFilter {
                budget_id: BudgetIdFilter::Equals("budget-del".to_owned()),
                spend: SpendRange::default(),
            });
            // Delete the budget on a separate connection before the batch commits.
            deleter
                .execute(
                    "DELETE FROM \"LiteLLM_BudgetTable\" WHERE budget_id = 'budget-del'",
                    &[],
                )
                .await
                .unwrap();
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_spend(
        &client,
        "LiteLLM_EndUserTable",
        "user_id",
        "enduser-del",
        0.0_f64,
    )
    .await;
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn missing_single_reset_aborts_batch() {
    let (fixture, store, client) = schema().await;
    let result = spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.teams
                .queue_spend_reset("missing-team", None, Decimal::from(3_i32));
            Ok(())
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(UnitOfWorkError::Commit(CommitError::MissingResetTarget {
            table: "LiteLLM_TeamTable"
        }))
    ));
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn total_spend_and_unrelated_columns_untouched() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute("INSERT INTO \"LiteLLM_UserTable\" (user_id, spend, total_spend) VALUES ('user-a', 10, 999); INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, label) VALUES ('token-a', 10, 'kept');")
        .await
        .unwrap();
    spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.users
                .queue_spend_reset("user-a", None, Decimal::from(3_i32));
            unit.keys
                .queue_spend_reset("token-a", None, Decimal::from(3_i32));
            Ok(())
        },
    )
    .await
    .unwrap();
    let total_spend: f64 = client
        .query_one(
            "SELECT total_spend FROM \"LiteLLM_UserTable\" WHERE user_id = 'user-a'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!((total_spend - 999.0_f64).abs() < f64::EPSILON);
    let label: String = client
        .query_one(
            "SELECT label FROM \"LiteLLM_VerificationToken\" WHERE token = 'token-a'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(label, "kept");
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn separate_units_commit_independently() {
    let (fixture, store, client) = schema().await;
    client
        .execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend) VALUES ('user-a', 10)",
            &[],
        )
        .await
        .unwrap();
    spend_reset_unit_of_work(
        || SpendResetBatch {
            store: store.clone(),
        },
        async move |unit: &mut SpendResetUnitOfWork| -> Result<(), TestError> {
            unit.users
                .queue_spend_reset("user-a", None, Decimal::from(3_i32));
            Ok(())
        },
    )
    .await
    .unwrap();
    let second = budget_cascade_unit_of_work(
        || BudgetCascadeBatch {
            store: store.clone(),
        },
        async move |unit: &mut BudgetCascadeUnitOfWork| -> Result<(), TestError> {
            unit.organizations.queue_spend_zero(LinkedResetFilter {
                budget_id: BudgetIdFilter::Equals("whatever".to_owned()),
                spend: SpendRange::default(),
            });
            Err(TestError)
        },
    )
    .await;
    assert!(matches!(second, Err(UnitOfWorkError::Body(TestError))));
    assert_spend(&client, "LiteLLM_UserTable", "user_id", "user-a", 7.0_f64).await;
    finish(fixture, store, client).await;
}
