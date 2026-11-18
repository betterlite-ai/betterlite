use rstest::rstest;

use crate::auth_read::budget_failure::{ExpectedLookupMiss, log_budget_lookup_failure};

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
        text.contains("postgres://REDACTED@db.internal:5432/app"),
        "{text}"
    );
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
