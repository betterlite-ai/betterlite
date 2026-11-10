#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use betterlite_db::database_tls::pool_policy::{
    BINARY_ENV, ENABLED_ENV, MAX_CLIENT_CONN_ENV, MAX_DB_CONNECTIONS_ENV, PORT_ENV,
    PgBouncerSettings, SessionRole, pool_limit,
};
use rstest::rstest;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};
use crate::types::Config;

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
    .database_tls
}

fn load_all(source: &str, pairs: &[(&str, &str)]) -> Config {
    ConfigLoader {
        lookup: variables(pairs),
    }
    .load(source)
    .expect("config should load")
}

fn load_error(pairs: &[(&str, &str)]) -> ConfigError {
    ConfigLoader {
        lookup: variables(pairs),
    }
    .load("model_list: []\n")
    .expect_err("config should fail")
}

fn logged(pairs: &[(&str, &str)]) -> (Settings, String) {
    let log = NamedTempFile::new().expect("a log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().expect("a log handle")))
        .finish();
    let settings = with_default(subscriber, || load(pairs));
    let output = fs::read_to_string(log.path()).expect("the captured log");
    (settings, output)
}

#[test]
fn pool_env_defaults() {
    let settings = load(&[]);
    assert_eq!(settings.pgbouncer, PgBouncerSettings::default());
    assert!(!settings.pgbouncer.enabled);
    assert_eq!(settings.pgbouncer.port, 6432);
    assert_eq!(settings.pgbouncer.max_db_connections, 20);
    assert_eq!(settings.pgbouncer.max_client_conn, 1000);
    assert_eq!(settings.pgbouncer.binary, "pgbouncer");
    let blank = load(&[
        (ENABLED_ENV, ""),
        (PORT_ENV, ""),
        (MAX_DB_CONNECTIONS_ENV, ""),
        (MAX_CLIENT_CONN_ENV, ""),
        (BINARY_ENV, ""),
    ]);
    assert_eq!(blank.pgbouncer, PgBouncerSettings::default());
}

#[test]
fn reads_every_pool_env_setting() {
    let settings = load(&[
        (ENABLED_ENV, "true"),
        (PORT_ENV, "7000"),
        (MAX_DB_CONNECTIONS_ENV, "30"),
        (MAX_CLIENT_CONN_ENV, "40"),
        (BINARY_ENV, "/usr/local/bin/pgbouncer"),
    ]);
    let pool = settings.pgbouncer;
    assert!(pool.enabled);
    assert_eq!(pool.port, 7000);
    assert_eq!(pool.max_db_connections, 30);
    assert_eq!(pool.max_client_conn, 40);
    assert_eq!(pool.binary, "/usr/local/bin/pgbouncer");
}

#[test]
fn pool_env_lowercase_names_are_read() {
    let settings = load(&[
        ("litellm_pgbouncer_enabled", "true"),
        ("litellm_pgbouncer_port", "7000"),
    ]);
    assert!(settings.pgbouncer.enabled);
    assert_eq!(settings.pgbouncer.port, 7000);
}

#[rstest]
#[case::true_word("true", true)]
#[case::upper("TRUE", true)]
#[case::one("1", true)]
#[case::yes("yes", true)]
#[case::on("on", true)]
#[case::false_word("false", false)]
#[case::zero("0", false)]
#[case::no("no", false)]
#[case::off("off", false)]
#[case::blank("", false)]
fn pool_enabled_boolean_values(#[case] text: &str, #[case] expected: bool) {
    let settings = load(&[(ENABLED_ENV, text)]);
    assert_eq!(settings.pgbouncer.enabled, expected);
}

#[rstest]
#[case::enabled_word(ENABLED_ENV, "maybe")]
#[case::port_word(PORT_ENV, "abc")]
#[case::port_fraction(PORT_ENV, "1.5")]
#[case::max_db_word(MAX_DB_CONNECTIONS_ENV, "ten")]
#[case::max_client_word(MAX_CLIENT_CONN_ENV, "many")]
fn invalid_pool_settings_stop_loading(#[case] name: &str, #[case] value: &str) {
    let error = load_error(&[(name, value)]);
    assert!(
        matches!(error, ConfigError::Environment { .. }),
        "{error:?}"
    );
}

#[rstest]
#[case::port_zero(PORT_ENV, "0")]
#[case::port_too_large(PORT_ENV, "65536")]
#[case::db_zero(MAX_DB_CONNECTIONS_ENV, "0")]
#[case::client_zero(MAX_CLIENT_CONN_ENV, "0")]
fn invalid_pool_ranges_stop_loading(#[case] name: &str, #[case] value: &str) {
    let error = load_error(&[(name, value)]);
    assert!(
        matches!(error, ConfigError::InvalidScalar { .. }),
        "{error:?}"
    );
}

#[test]
fn native_writer_pool_cap() {
    let capped = load_all(
        "general_settings:\n  database_connection_pool_limit: 50\n",
        &[(ENABLED_ENV, "true"), (MAX_DB_CONNECTIONS_ENV, "20")],
    );
    let limit = capped.sections.database_urls.pool_limit;
    assert_eq!(limit, 50);
    assert_eq!(
        pool_limit(
            SessionRole::Writer,
            limit,
            &capped.sections.database_tls.pgbouncer
        ),
        20
    );
    let smaller = load_all(
        "general_settings:\n  database_connection_pool_limit: 5\n",
        &[(ENABLED_ENV, "true"), (MAX_DB_CONNECTIONS_ENV, "20")],
    );
    let smaller_limit = smaller.sections.database_urls.pool_limit;
    assert_eq!(
        pool_limit(
            SessionRole::Writer,
            smaller_limit,
            &smaller.sections.database_tls.pgbouncer
        ),
        5
    );
}

#[test]
fn native_disabled_pool_unchanged() {
    let config = load_all(
        "general_settings:\n  database_connection_pool_limit: 50\n",
        &[(MAX_DB_CONNECTIONS_ENV, "20")],
    );
    let limit = config.sections.database_urls.pool_limit;
    assert_eq!(
        pool_limit(
            SessionRole::Writer,
            limit,
            &config.sections.database_tls.pgbouncer
        ),
        50
    );
}

#[test]
fn native_reader_and_migration_uncapped() {
    let config = load_all(
        "general_settings:\n  database_connection_pool_limit: 50\n",
        &[(ENABLED_ENV, "true"), (MAX_DB_CONNECTIONS_ENV, "20")],
    );
    let limit = config.sections.database_urls.pool_limit;
    let pool = &config.sections.database_tls.pgbouncer;
    assert_eq!(pool_limit(SessionRole::Reader, limit, pool), 50);
    assert_eq!(pool_limit(SessionRole::Migration, limit, pool), 50);
}

#[test]
fn native_ignored_options_warn_once() {
    let (settings, log) = logged(&[
        (PORT_ENV, "7000"),
        (MAX_CLIENT_CONN_ENV, "50"),
        (BINARY_ENV, "/usr/local/bin/pgbouncer"),
    ]);
    assert_eq!(log.matches("WARN").count(), 1, "{log}");
    assert!(log.contains(PORT_ENV), "{log}");
    assert!(log.contains(MAX_CLIENT_CONN_ENV), "{log}");
    assert!(log.contains(BINARY_ENV), "{log}");
    assert_eq!(settings.pgbouncer.port, 7000);
    let (_, quiet) = logged(&[(ENABLED_ENV, "true")]);
    assert!(!quiet.contains("WARN"), "{quiet}");
}
