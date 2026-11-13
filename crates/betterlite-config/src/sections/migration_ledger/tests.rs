#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use betterlite_db::migration_ledger::environment::{migration_environment, prisma_schema_param};
use betterlite_db::migration_ledger::timeouts::{
    MIGRATION_LOCK_TIMEOUT_ENV_VAR, PRISMA_COMMAND_TIMEOUT_ENV_VAR, PRISMA_DEPLOY_TIMEOUT_ENV_VAR,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use super::Settings;
use crate::loader::ConfigLoader;

fn variables(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(pairs: &[(&str, &str)]) -> Settings {
    ConfigLoader {
        lookup: variables(pairs),
    }
    .load("model_list: []\n")
    .expect("config should load")
    .sections
    .migration_ledger
}

fn configured_database_url(settings: &Settings) -> Option<&str> {
    settings
        .inner
        .database_url
        .as_ref()
        .map(SecretString::expose_secret)
}

fn configured_direct_url(settings: &Settings) -> Option<&str> {
    settings
        .inner
        .direct_url
        .as_ref()
        .map(SecretString::expose_secret)
}

fn logged(pairs: &[(&str, &str)]) -> (Settings, String) {
    let log = NamedTempFile::new().expect("a log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("a log handle")))
        .finish();
    let loaded = with_default(subscriber, || load(pairs));
    let output = fs::read_to_string(log.path()).expect("the captured log");
    (loaded, output)
}

#[test]
fn lock_timeout_default() {
    let loaded = load(&[]);
    assert_eq!(
        loaded.inner.migration_lock_timeout,
        Duration::from_secs(600)
    );
    let blank = load(&[(MIGRATION_LOCK_TIMEOUT_ENV_VAR, "")]);
    assert_eq!(blank.inner.migration_lock_timeout, Duration::from_secs(600));
}

#[test]
fn positive_fractional_timeout() {
    let loaded = load(&[(MIGRATION_LOCK_TIMEOUT_ENV_VAR, "0.25")]);
    assert_eq!(
        loaded.inner.migration_lock_timeout,
        Duration::from_millis(250)
    );
    let padded = load(&[(MIGRATION_LOCK_TIMEOUT_ENV_VAR, " 12.5 ")]);
    assert_eq!(
        padded.inner.migration_lock_timeout,
        Duration::from_millis(12_500)
    );
}

#[test]
fn invalid_timeout_warns() {
    let (loaded, log) = logged(&[(MIGRATION_LOCK_TIMEOUT_ENV_VAR, "soon")]);
    assert_eq!(
        loaded.inner.migration_lock_timeout,
        Duration::from_secs(600)
    );
    assert!(log.contains("is not a number"), "{log}");
    assert!(log.contains(MIGRATION_LOCK_TIMEOUT_ENV_VAR), "{log}");
    assert!(log.contains("fallback_seconds=600"), "{log}");
}

#[test]
fn command_timeout_default_sixty() {
    let loaded = load(&[]);
    assert_eq!(loaded.inner.prisma_command_timeout, Duration::from_secs(60));
    let configured = load(&[(PRISMA_COMMAND_TIMEOUT_ENV_VAR, "90")]);
    assert_eq!(
        configured.inner.prisma_command_timeout,
        Duration::from_secs(90)
    );
}

#[test]
fn deploy_timeout_default_six_hundred() {
    let loaded = load(&[]);
    assert_eq!(
        loaded.inner.prisma_migrate_deploy_timeout,
        Duration::from_secs(600)
    );
}

#[test]
fn absent_deploy_inherits_larger_command() {
    let loaded = load(&[(PRISMA_COMMAND_TIMEOUT_ENV_VAR, "900")]);
    assert_eq!(
        loaded.inner.prisma_migrate_deploy_timeout,
        Duration::from_mins(15)
    );
    let shorter = load(&[(PRISMA_COMMAND_TIMEOUT_ENV_VAR, "30")]);
    assert_eq!(
        shorter.inner.prisma_migrate_deploy_timeout,
        Duration::from_secs(600)
    );
}

#[test]
fn explicit_deploy_overrides_command() {
    let longer = load(&[
        (PRISMA_COMMAND_TIMEOUT_ENV_VAR, "900"),
        (PRISMA_DEPLOY_TIMEOUT_ENV_VAR, "1200"),
    ]);
    assert_eq!(
        longer.inner.prisma_migrate_deploy_timeout,
        Duration::from_mins(20)
    );
    let shorter = load(&[
        (PRISMA_COMMAND_TIMEOUT_ENV_VAR, "900"),
        (PRISMA_DEPLOY_TIMEOUT_ENV_VAR, "300"),
    ]);
    assert_eq!(
        shorter.inner.prisma_migrate_deploy_timeout,
        Duration::from_secs(300)
    );
}

#[test]
fn invalid_present_deploy_uses_six_hundred() {
    let (loaded, log) = logged(&[
        (PRISMA_COMMAND_TIMEOUT_ENV_VAR, "900"),
        (PRISMA_DEPLOY_TIMEOUT_ENV_VAR, "abc"),
    ]);
    assert_eq!(
        loaded.inner.prisma_migrate_deploy_timeout,
        Duration::from_secs(600)
    );
    assert_eq!(loaded.inner.prisma_command_timeout, Duration::from_mins(15));
    assert!(log.contains(PRISMA_DEPLOY_TIMEOUT_ENV_VAR), "{log}");
}

#[rstest]
#[case::text("abc", "is not a number")]
#[case::blank("", "is not a number")]
#[case::zero("0", "is not a finite positive number")]
#[case::negative("-4", "is not a finite positive number")]
#[case::infinite("inf", "is not a finite positive number")]
#[case::not_a_number("NaN", "is not a finite positive number")]
fn timeout_warning_categories(#[case] value: &str, #[case] expected: &str) {
    let (loaded, log) = logged(&[(MIGRATION_LOCK_TIMEOUT_ENV_VAR, value)]);
    assert_eq!(
        loaded.inner.migration_lock_timeout,
        Duration::from_secs(600)
    );
    assert!(log.contains(expected), "{log}");
    let other = if expected == "is not a number" {
        "is not a finite positive number"
    } else {
        "is not a number"
    };
    assert!(!log.contains(other), "{log}");
}

#[test]
fn direct_url_schema_override() {
    let database = "postgresql://writer/db?schema=tenant_a&sslmode=require";
    let direct = "postgresql://direct:6543/db?pgbouncer=true&schema=pooled&connection_limit=5";
    let loaded = load(&[("DATABASE_URL", database), ("DIRECT_URL", direct)]);
    let effective = migration_environment(
        configured_database_url(&loaded),
        configured_direct_url(&loaded),
    )
    .expect("the URLs parse")
    .expect("a migration URL is present");
    assert_eq!(
        effective,
        "postgresql://direct:6543/db?pgbouncer=true&connection_limit=5&schema=tenant_a"
    );
}

#[test]
fn missing_url_passthrough() {
    let database = "postgresql://writer/db?schema=tenant_a";
    let configured = load(&[("DATABASE_URL", database)]);
    assert!(configured.inner.direct_url.is_none());
    assert_eq!(
        migration_environment(configured_database_url(&configured), None).expect("the URL parses"),
        Some(database.to_owned())
    );
    let absent = load(&[]);
    assert!(absent.inner.database_url.is_none());
    assert_eq!(
        migration_environment(configured_database_url(&absent), None).expect("nothing to parse"),
        None
    );
    let blank = load(&[("DATABASE_URL", ""), ("DIRECT_URL", "")]);
    assert!(blank.inner.database_url.is_none());
    assert!(blank.inner.direct_url.is_none());
}

#[test]
fn first_schema_value() {
    let database = "postgresql://writer/db?schema=first&schema=second";
    let loaded = load(&[("DATABASE_URL", database)]);
    assert_eq!(
        prisma_schema_param(configured_database_url(&loaded).expect("the URL is configured"))
            .expect("the URL parses")
            .as_deref(),
        Some("first")
    );
    assert_eq!(
        prisma_schema_param("postgresql://writer/db?schema=&schema=second")
            .expect("the URL parses")
            .as_deref(),
        Some("second")
    );
    assert_eq!(
        prisma_schema_param("postgresql://writer/db?other=1").expect("the URL parses"),
        None
    );
    assert_eq!(
        prisma_schema_param("postgresql://writer/db?schema=&other=1").expect("the URL parses"),
        None
    );
}
