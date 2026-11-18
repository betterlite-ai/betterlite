use rstest::rstest;
use tokio_postgres::error::{DbError, SqlState};

use crate::auth_read::budget_failure::{ExpectedLookupMiss, log_budget_lookup_failure};

use super::postgres::{close, database};
use super::support::{Logs, redactor};

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct NamedFailure {
    message: String,
}

impl NamedFailure {
    fn new(message: &str) -> Self {
        Self {
            message: message.to_owned(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the budget lookup failed")]
struct ChainedFailure {
    #[source]
    source: NamedFailure,
}

#[derive(Debug, thiserror::Error)]
#[error("the budget lookup failed")]
struct LookupFailure {
    #[source]
    source: tokio_postgres::Error,
}

fn logged(failure: tokio_postgres::Error) -> String {
    let (logs, _guard) = Logs::capture();
    log_budget_lookup_failure(&redactor(), "user", &LookupFailure { source: failure });
    logs.text()
}

#[test]
fn empty_exact_exception_suppressed() {
    let (logs, _guard) = Logs::capture();
    log_budget_lookup_failure(
        &redactor(),
        "user",
        &ExpectedLookupMiss {
            message: String::new(),
        },
    );
    assert_eq!(logs.text(), "");
}

#[test]
fn empty_subclass_still_error() {
    let (logs, _guard) = Logs::capture();
    log_budget_lookup_failure(&redactor(), "user", &NamedFailure::new(""));
    let text = logs.text();
    assert!(text.contains("ERROR"), "{text}");
}

#[test]
fn nonempty_generic_exception_error() {
    let (logs, _guard) = Logs::capture();
    log_budget_lookup_failure(
        &redactor(),
        "user",
        &ExpectedLookupMiss {
            message: "user not found".to_owned(),
        },
    );
    let text = logs.text();
    assert!(text.contains("ERROR"), "{text}");
    assert!(text.contains("user not found"), "{text}");
}

#[test]
fn budget_failure_not_warning() {
    let (logs, _guard) = Logs::capture();
    log_budget_lookup_failure(
        &redactor(),
        "team",
        &ExpectedLookupMiss {
            message: "the connection dropped".to_owned(),
        },
    );
    let text = logs.text();
    assert!(text.contains("ERROR"), "{text}");
    assert!(!text.contains("WARN"), "{text}");
}

#[rstest]
#[case::column("CoLuMn missing from litellm_budgettable")]
#[case::schema("SCHEMA drift detected")]
#[case::does_not_exist("relation DOES NOT EXIST")]
#[case::prisma("Prisma client failed")]
#[case::migrate("please MIGRATE the database")]
#[test]
fn each_schema_hint_substring_casefold(#[case] message: &str) {
    let (logs, _guard) = Logs::capture();
    log_budget_lookup_failure(&redactor(), "user", &NamedFailure::new(message));
    let text = logs.text();
    assert!(text.contains("betterlite-db-admin migrate"), "{text}");
}

#[test]
fn unrelated_error_no_hint() {
    let (logs, _guard) = Logs::capture();
    let failure = NamedFailure::new("connection refused by the pool");
    log_budget_lookup_failure(&redactor(), "user", &failure);
    let text = logs.text();
    assert!(!text.contains("betterlite-db-admin migrate"), "{text}");
    assert!(text.contains("connection refused by the pool"), "{text}");
}

#[test]
fn logging_does_not_mutate_or_retry() {
    let (logs, _guard) = Logs::capture();
    let message = String::from("the schema drifted");
    let failure = NamedFailure {
        message: message.clone(),
    };
    log_budget_lookup_failure(&redactor(), "user", &failure);
    let text = logs.text();
    assert_eq!(message, "the schema drifted");
    assert_eq!(text.matches("ERROR").count(), 1);
    assert!(text.contains("budget lookup failed for user"), "{text}");
    assert!(text.contains("the cache will not be populated"), "{text}");
    assert!(
        text.contains("each request will reach the database"),
        "{text}"
    );
}

#[test]
fn hint_uses_native_migration_command() {
    let (logs, _guard) = Logs::capture();
    log_budget_lookup_failure(
        &redactor(),
        "user",
        &NamedFailure::new("column does not exist"),
    );
    let text = logs.text();
    assert!(text.contains("betterlite-db-admin migrate"), "{text}");
    assert!(!text.contains("prisma db push"), "{text}");
    assert!(!text.contains("prisma migrate deploy"), "{text}");
}

#[test]
fn lookup_diagnostics_redact_credentials() {
    let (logs, _guard) = Logs::capture();
    let failure =
        NamedFailure::new("could not connect: postgres://dbuser:dbpass123@db.internal:5432/app");
    log_budget_lookup_failure(&redactor(), "user", &failure);
    let text = logs.text();
    assert!(!text.contains("dbpass123"), "{text}");
    assert!(
        text.contains("postgres://REDACTED@REDACTED:5432/app"),
        "{text}"
    );
    assert!(!text.contains("db.internal"), "{text}");
    assert!(!text.contains("betterlite-db-admin migrate"), "{text}");
}

#[test]
fn nested_source_message_and_hint_logged() {
    let (logs, _guard) = Logs::capture();
    let failure = ChainedFailure {
        source: NamedFailure::new("schema drift in litellm_budgettable"),
    };
    log_budget_lookup_failure(&redactor(), "user", &failure);
    let text = logs.text();
    assert!(text.contains("the budget lookup failed"), "{text}");
    assert!(
        text.contains("schema drift in litellm_budgettable"),
        "{text}"
    );
    assert!(text.contains("betterlite-db-admin migrate"), "{text}");
}

#[tokio::test]
async fn unique_violation_logs_sqlstate_and_constraint() {
    let (fixture, pool) = database().await;
    let text = {
        let client = pool.get().await.expect("the database connection");
        let insert =
            "INSERT INTO \"LiteLLM_VerificationToken\" (token) VALUES ('opaque-customer-token')";
        client.batch_execute(insert).await.expect("the token row");
        let failure = client
            .batch_execute(insert)
            .await
            .expect_err("the duplicate token violates the primary key");
        assert_eq!(
            failure.as_db_error().and_then(DbError::constraint),
            Some("LiteLLM_VerificationToken_pkey")
        );
        logged(failure)
    };
    assert!(text.contains("23505"), "{text}");
    assert!(text.contains("LiteLLM_VerificationToken_pkey"), "{text}");
    assert!(!text.contains("duplicate key value"), "{text}");
    assert!(!text.contains("opaque-customer-token"), "{text}");
    close(fixture, pool).await;
}

#[tokio::test]
async fn undefined_column_keeps_migrate_hint() {
    let (fixture, pool) = database().await;
    let text = {
        let client = pool.get().await.expect("the database connection");
        let failure = client
            .query("SELECT \"missing_column\" FROM \"LiteLLM_UserTable\"", &[])
            .await
            .expect_err("the missing column is undefined");
        assert_eq!(failure.code().map(SqlState::code), Some("42703"));
        logged(failure)
    };
    // PostgreSQL 16 sends no column diagnostic with 42703; the not-null case covers that field.
    assert!(text.contains("42703"), "{text}");
    assert!(text.contains("betterlite-db-admin migrate"), "{text}");
    assert!(!text.contains("does not exist"), "{text}");
    close(fixture, pool).await;
}

#[tokio::test]
async fn not_null_violation_logs_table_and_column() {
    let (fixture, pool) = database().await;
    let text = {
        let client = pool.get().await.expect("the database connection");
        let failure = client
            .execute(
                "INSERT INTO \"LiteLLM_UserTable\" (\"user_email\") VALUES ('leak@example.com')",
                &[],
            )
            .await
            .expect_err("the user row needs an identifier");
        assert_eq!(failure.code().map(SqlState::code), Some("23502"));
        logged(failure)
    };
    assert!(text.contains("23502"), "{text}");
    assert!(text.contains("LiteLLM_UserTable"), "{text}");
    assert!(text.contains("user_id"), "{text}");
    assert!(!text.contains("null value in column"), "{text}");
    assert!(!text.contains("leak@example.com"), "{text}");
    close(fixture, pool).await;
}
