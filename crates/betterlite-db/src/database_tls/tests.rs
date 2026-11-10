#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Write as _;
use std::time::Duration;

use rstest::rstest;
use tokio::task::JoinSet;
use tokio_postgres::config::SslMode;
use url::Url;

use super::options::tls_config::add_native_roots;
use super::options::{ConnectionOptionsError, native_connection_options};
use super::pool_policy::{
    BINARY_ENV, DEFAULT_BINARY, DEFAULT_MAX_CLIENT_CONN, DEFAULT_MAX_DB_CONNECTIONS, DEFAULT_PORT,
    ENABLED_ENV, MAX_CLIENT_CONN_ENV, MAX_DB_CONNECTIONS_ENV, PORT_ENV, PgBouncerSettings,
    PoolEnvironment, PoolSettingsError, SessionRole, from_env, pool_limit,
};
use super::translation::{
    TranslateError, libpq_tls_params, strip_prisma_query_params, translate_libpq_ssl_params,
};
use crate::database_url_policy::url_codec::urlencode_pairs_quote;

mod certificates;
mod front;
mod server;

use front::{Front, Mode};

fn write_file(contents: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("a temporary file");
    file.write_all(contents.as_bytes()).expect("file contents");
    file
}

fn path_of(file: &tempfile::NamedTempFile) -> &str {
    file.path().to_str().expect("a UTF-8 path")
}

fn translate(url: &str) -> (String, Vec<(String, String, u16)>) {
    let mut calls = Vec::new();
    let translated = translate_libpq_ssl_params(url, |path, host, port| {
        calls.push((path.to_owned(), host.to_owned(), port));
        path.to_owned()
    })
    .expect("the URL translates");
    (translated, calls)
}

fn strip(url: &str) -> String {
    strip_prisma_query_params(url).expect("the URL strips")
}

fn client_url(server: &server::Server, host: &str, port: u16, params: &[(&str, &str)]) -> String {
    let parsed = Url::parse(&server.url).expect("the upstream URL");
    let pairs: Vec<(String, String)> = params
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    let query = urlencode_pairs_quote(&pairs);
    format!(
        "postgresql://{}:{}@{host}:{port}{}?{query}",
        parsed.username(),
        parsed.password().unwrap_or_default(),
        parsed.path()
    )
}

async fn connect_and_query(url: &str) {
    let options = native_connection_options(url).expect("connection options");
    let (client, connection) = options
        .config
        .connect(options.tls)
        .await
        .expect("a database connection");
    let mut drivers = JoinSet::new();
    drivers.spawn(connection);
    let row = client.query_one("SELECT 1", &[]).await.expect("a query");
    assert_eq!(row.get::<_, i32>(0), 1_i32);
    drop(client);
    drivers.shutdown().await;
}

async fn rejected(url: &str) {
    let options = native_connection_options(url).expect("connection options");
    assert!(
        options.config.connect(options.tls).await.is_err(),
        "the connection must be rejected"
    );
}

#[rstest]
#[case::sslmode_require("postgresql://u:p@h:5432/db?sslmode=require&application_name=app")]
#[case::no_query("postgresql://u@h/db")]
#[case::sslmode_prefer("postgresql://u@h/db?sslmode=prefer")]
fn untouched_tls_url(#[case] url: &str) {
    let (translated, calls) = translate(url);
    assert_eq!(translated, url);
    assert!(calls.is_empty());
}

#[test]
fn verify_modes_rewritten() {
    let (translated, _) =
        translate("postgresql://u:p@host:6543/db?sslmode=verify-ca&application_name=app");
    assert_eq!(
        translated,
        "postgresql://u:p@host:6543/db?sslmode=require&application_name=app&sslaccept=strict"
    );
    let (duplicated, _) = translate("postgresql://h/db?sslmode=require&sslmode=verify-full");
    assert_eq!(
        duplicated,
        "postgresql://h/db?sslmode=require&sslmode=require&sslaccept=strict"
    );
}

#[test]
fn blank_pinned_tls_values() {
    let (pinned_accept, _) = translate("postgresql://h/db?sslmode=verify-full&sslaccept=");
    assert_eq!(
        pinned_accept,
        "postgresql://h/db?sslmode=require&sslaccept="
    );
    let (blank_root, root_calls) = translate("postgresql://h/db?sslrootcert=&sslmode=verify-full");
    assert_eq!(
        blank_root,
        "postgresql://h/db?sslmode=require&sslcert=&sslaccept=strict"
    );
    assert_eq!(root_calls, vec![(String::new(), "h".to_owned(), 5432)]);
    let (blank_cert, cert_calls) = translate("postgresql://h/db?sslcert=&sslrootcert=ca.pem");
    assert_eq!(blank_cert, "postgresql://h/db?sslcert=&sslaccept=strict");
    assert!(cert_calls.is_empty());
}

#[test]
fn duplicate_root_pairs() {
    let (translated, calls) = translate(
        "postgresql://h/db?sslmode=verify-full&sslrootcert=a.pem&sslrootcert=b.pem&application_name=a&application_name=b",
    );
    assert_eq!(
        translated,
        "postgresql://h/db?sslmode=require&application_name=a&application_name=b&sslcert=a.pem&sslcert=b.pem&sslaccept=strict"
    );
    assert_eq!(
        calls,
        vec![
            ("a.pem".to_owned(), "h".to_owned(), 5432),
            ("b.pem".to_owned(), "h".to_owned(), 5432),
        ]
    );
}

#[test]
fn root_without_mode() {
    let (translated, calls) = translate("postgresql://u@host/db?sslrootcert=ca.pem");
    assert_eq!(
        translated,
        "postgresql://u@host/db?sslcert=ca.pem&sslaccept=strict"
    );
    assert_eq!(calls, vec![("ca.pem".to_owned(), "host".to_owned(), 5432)]);
}

#[test]
fn explicit_cert_precedence() {
    let (translated, calls) = translate(
        "postgresql://u@host/db?sslrootcert=root.pem&sslcert=pinned.pem&sslmode=verify-full",
    );
    assert_eq!(
        translated,
        "postgresql://u@host/db?sslcert=pinned.pem&sslmode=require&sslaccept=strict"
    );
    assert!(calls.is_empty());
}

#[test]
fn resolver_host_port_arguments() {
    let (translated, upper_calls) =
        translate("postgresql://user@DB.Example:6543/db?sslrootcert=/ca.pem");
    assert_eq!(
        translated,
        "postgresql://user@DB.Example:6543/db?sslcert=%2Fca.pem&sslaccept=strict"
    );
    assert_eq!(
        upper_calls,
        vec![("/ca.pem".to_owned(), "db.example".to_owned(), 6543)]
    );
    let (without_netloc, netloc_calls) = translate("postgresql:///db?sslrootcert=ca");
    assert_eq!(without_netloc, "postgresql:/db?sslcert=ca&sslaccept=strict");
    assert_eq!(netloc_calls, vec![("ca".to_owned(), String::new(), 5432)]);
    let (zero_port, zero_calls) = translate("postgresql://h:0/db?sslrootcert=ca");
    assert_eq!(zero_calls, vec![("ca".to_owned(), "h".to_owned(), 5432)]);
    let (empty_port, empty_calls) = translate("postgresql://h:/db?sslrootcert=ca");
    assert_eq!(empty_calls, vec![("ca".to_owned(), "h".to_owned(), 5432)]);
    assert!(zero_port.contains(":0/"));
    assert!(empty_port.contains(":/"));
}

#[rstest]
#[case::not_a_number("notaport")]
#[case::surrounding_space(" 5432")]
#[case::plus_sign("+5432")]
#[case::digit_underscore("5_432")]
#[case::negative_zero("-0")]
fn malformed_port_error(#[case] port: &str) {
    let url = format!("postgresql://host:{port}/db?sslrootcert=ca.pem");
    let error = translate_libpq_ssl_params(&url, |path, _, _| path.to_owned())
        .expect_err("a malformed port must fail");
    assert!(
        matches!(error, TranslateError::MalformedPort { .. }),
        "{error:?}"
    );
}

#[test]
fn port_errors_only_when_a_root_resolves() {
    let out_of_range = translate_libpq_ssl_params(
        "postgresql://host:99999/db?sslrootcert=ca.pem",
        |path, _, _| path.to_owned(),
    )
    .expect_err("an out-of-range port must fail");
    assert!(
        matches!(out_of_range, TranslateError::PortOutOfRange { .. }),
        "{out_of_range:?}"
    );
    let (translated, calls) = translate("postgresql://host:notaport/db?sslmode=verify-full");
    assert_eq!(
        translated,
        "postgresql://host:notaport/db?sslmode=require&sslaccept=strict"
    );
    assert!(calls.is_empty());
}

#[test]
fn strip_exact_libpq_allowlist() {
    let url = "postgresql://u@h/db?connection_limit=10&pool_timeout=60&socket_timeout=5&max_idle_connection_lifetime=60&schema=tenant&pgbouncer=true&sslmode=require&sslmode=prefer&application_name=app&connect_timeout=5&client_encoding=UTF8&options=-c%20x%3D1&service=svc&gssencmode=disable&krbsrvname=postgres&target_session_attrs=any&sslaccept=accept_invalid_certs";
    assert_eq!(
        strip(url),
        "postgresql://u@h/db?sslmode=require&sslmode=prefer&application_name=app&connect_timeout=5&client_encoding=UTF8&options=-c%20x%3D1&service=svc&gssencmode=disable&krbsrvname=postgres&target_session_attrs=any"
    );
}

#[test]
fn strip_blank_pairs_and_percent_spaces() {
    assert_eq!(
        strip(
            "postgresql://u@h/db?application_name=my%20app&connect_timeout=&schema=tenant&sslaccept="
        ),
        "postgresql://u@h/db?application_name=my%20app"
    );
    assert_eq!(
        strip("postgresql://u@h/db?application_name=my+app"),
        "postgresql://u@h/db?application_name=my%20app"
    );
    assert_eq!(strip("postgresql://u@h/db"), "postgresql://u@h/db");
    assert_eq!(strip("postgresql://u@h/db?"), "postgresql://u@h/db?");
    assert_eq!(strip("postgresql:///db?schema=tenant"), "postgresql:/db");
}

#[test]
fn sslkey_client_cert_bypass() {
    assert_eq!(
        strip(
            "postgresql://u@h/db?sslcert=client.pem&sslkey=client.key&sslmode=verify-full&sslaccept=strict"
        ),
        "postgresql://u@h/db?sslcert=client.pem&sslkey=client.key&sslmode=verify-full"
    );
    let pairs = vec![
        ("sslcert".to_owned(), "client.pem".to_owned()),
        ("sslkey".to_owned(), "client.key".to_owned()),
    ];
    assert_eq!(libpq_tls_params(&pairs, Some("strict")), pairs);
}

#[test]
fn absent_sslaccept_drops_ca() {
    assert_eq!(
        strip("postgresql://u@h/db?sslcert=ca.pem&sslmode=require"),
        "postgresql://u@h/db?sslmode=require"
    );
    let pairs = vec![("sslcert".to_owned(), "ca.pem".to_owned())];
    assert!(libpq_tls_params(&pairs, None).is_empty());
}

#[test]
fn accept_invalid_certs_drops_ca() {
    assert_eq!(
        strip("postgresql://u@h/db?sslcert=ca.pem&sslmode=require&sslaccept=accept_invalid_certs"),
        "postgresql://u@h/db?sslmode=require"
    );
}

#[test]
fn disable_drops_prisma_ca() {
    assert_eq!(
        strip("postgresql://u@h/db?sslcert=ca.pem&sslmode=disable&sslaccept=strict"),
        "postgresql://u@h/db?sslmode=disable"
    );
    let pairs = vec![
        ("sslcert".to_owned(), "ca.pem".to_owned()),
        ("sslmode".to_owned(), "disable".to_owned()),
    ];
    assert_eq!(
        libpq_tls_params(&pairs, Some("strict")),
        vec![("sslmode".to_owned(), "disable".to_owned())]
    );
}

#[test]
fn strict_forces_verify_full() {
    assert_eq!(
        strip(
            "postgresql://u@h/db?sslcert=ca.pem&sslmode=prefer&sslaccept=strict&application_name=app"
        ),
        "postgresql://u@h/db?application_name=app&sslrootcert=ca.pem&sslmode=verify-full"
    );
}

#[test]
fn existing_root_wins() {
    assert_eq!(
        strip(
            "postgresql://u@h/db?sslcert=prisma-ca.pem&sslrootcert=explicit.pem&sslmode=require&sslaccept=strict"
        ),
        "postgresql://u@h/db?sslrootcert=explicit.pem&sslmode=verify-full"
    );
}

#[test]
fn first_sslmode_and_sslaccept() {
    assert_eq!(
        strip(
            "postgresql://u@h/db?sslmode=require&sslmode=disable&sslcert=ca.pem&sslaccept=strict&sslaccept=accept_invalid_certs"
        ),
        "postgresql://u@h/db?sslrootcert=ca.pem&sslmode=verify-full"
    );
    assert_eq!(
        strip("postgresql://u@h/db?sslcert=ca&sslaccept=&sslaccept=accept_invalid_certs"),
        "postgresql://u@h/db"
    );
    let pairs = vec![
        ("sslmode".to_owned(), "require".to_owned()),
        ("sslmode".to_owned(), "disable".to_owned()),
        ("sslcert".to_owned(), "ca".to_owned()),
    ];
    assert_eq!(
        libpq_tls_params(&pairs, Some("strict")),
        vec![
            ("sslrootcert".to_owned(), "ca".to_owned()),
            ("sslmode".to_owned(), "verify-full".to_owned()),
        ]
    );
}

#[test]
fn external_pooler_disables_prepared_statements() {
    let external =
        native_connection_options("postgresql://u:p@h:5432/db?sslmode=require&pgbouncer=true")
            .expect("connection options");
    assert!(external.disable_prepared_statements);
    let direct = native_connection_options("postgresql://u:p@h:5432/db?sslmode=disable")
        .expect("connection options");
    assert!(!direct.disable_prepared_statements);
    let error = native_connection_options("postgresql://u:p@h/db?pgbouncer=yes")
        .expect_err("an invalid pgbouncer flag must fail");
    assert!(error.to_string().contains("pgbouncer"));
}

#[test]
fn native_connection_options_keep_connection_params() {
    let options = native_connection_options(
        "postgresql://u:p@h:5432/db?application_name=app&connect_timeout=5&sslmode=disable",
    )
    .expect("connection options");
    assert_eq!(options.config.get_application_name(), Some("app"));
    assert_eq!(
        options.config.get_connect_timeout(),
        Some(&Duration::from_secs(5))
    );
}

#[test]
fn native_connection_options_debug_redacts_password() {
    let options = native_connection_options("postgresql://u:secret@h/db?sslmode=disable")
        .expect("connection options");
    let shown = format!("{options:?}");
    assert!(shown.contains("NativeConnectionOptions"), "{shown}");
    assert!(!shown.contains("secret"), "{shown}");
}

#[test]
fn tls_disabled_ignores_verification_settings() {
    let missing = tempfile::TempDir::new().expect("a temporary directory");
    let missing_path = missing.path().join("missing-ca.pem");
    let url = query_url(&[
        ("sslmode", "disable"),
        ("sslaccept", "strict"),
        ("sslrootcert", missing_path.to_str().expect("a UTF-8 path")),
    ]);
    let options = native_connection_options(&url).expect("disabled TLS loads no roots");
    assert_eq!(options.config.get_ssl_mode(), SslMode::Disable);
}

#[test]
fn prisma_ca_cert_is_loaded() {
    let missing = tempfile::TempDir::new().expect("a temporary directory");
    let missing_path = missing.path().join("missing-ca.pem");
    let url = query_url(&[
        ("sslmode", "verify-full"),
        ("sslcert", missing_path.to_str().expect("a UTF-8 path")),
    ]);
    let error = native_connection_options(&url).expect_err("the Prisma CA must be read");
    assert!(error.to_string().contains("missing-ca.pem"), "{error}");
}

#[test]
fn tls_client_identity_is_loaded() {
    let authority = certificates::authority();
    let certificate = write_file(&authority.leaf_pem);
    let key = write_file(&authority.key_pem);
    let url = query_url(&[
        ("sslmode", "require"),
        ("sslcert", path_of(&certificate)),
        ("sslkey", path_of(&key)),
    ]);
    native_connection_options(&url).expect("a PEM client identity");
}

#[test]
fn native_rejects_prisma_pkcs12_client_identity() {
    let identity = native_connection_options("postgresql://u@h/db?sslidentity=client.p12")
        .expect_err("sslidentity is not supported");
    assert!(identity.to_string().contains("sslidentity"), "{identity}");
    let key_only = native_connection_options("postgresql://u@h/db?sslkey=client.key")
        .expect_err("a key without a certificate is an error");
    assert!(key_only.to_string().contains("sslkey"), "{key_only}");
}

#[rstest]
#[case::allow("allow")]
#[case::blank("")]
#[case::unknown("bogus")]
fn native_unknown_sslmode_becomes_prefer(#[case] sslmode: &str) {
    let url = format!("postgresql://u@h/db?sslmode={sslmode}");
    let options =
        native_connection_options(&url).expect("Prisma reads an unknown sslmode as prefer");
    assert_eq!(options.config.get_ssl_mode(), SslMode::Prefer);
}

#[test]
fn native_last_duplicate_ssl_setting_wins() {
    let disabled = native_connection_options("postgresql://u@h/db?sslmode=require&sslmode=disable")
        .expect("connection options");
    assert_eq!(disabled.config.get_ssl_mode(), SslMode::Disable);
    let required = native_connection_options("postgresql://u@h/db?sslmode=disable&sslmode=require")
        .expect("connection options");
    assert_eq!(required.config.get_ssl_mode(), SslMode::Require);
}

fn pool_environment(pairs: &[(&str, &str)]) -> PoolEnvironment {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    from_env(&environment).expect("pool settings load")
}

fn pool_settings(pairs: &[(&str, &str)]) -> PgBouncerSettings {
    pool_environment(pairs).settings
}

fn pool_error(pairs: &[(&str, &str)]) -> PoolSettingsError {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    from_env(&environment).expect_err("pool settings fail")
}

#[test]
fn pool_env_defaults_load() {
    let outcome = pool_environment(&[]);
    assert_eq!(outcome.settings, PgBouncerSettings::default());
    assert_eq!(outcome.settings.port, DEFAULT_PORT);
    assert_eq!(
        outcome.settings.max_db_connections,
        DEFAULT_MAX_DB_CONNECTIONS
    );
    assert_eq!(outcome.settings.max_client_conn, DEFAULT_MAX_CLIENT_CONN);
    assert_eq!(outcome.settings.binary, DEFAULT_BINARY);
    assert!(outcome.ignored.is_empty());
}

#[test]
fn pool_env_reads_every_setting() {
    let settings = pool_settings(&[
        (ENABLED_ENV, "true"),
        (PORT_ENV, "7000"),
        (MAX_DB_CONNECTIONS_ENV, "30"),
        (MAX_CLIENT_CONN_ENV, "40"),
        (BINARY_ENV, "/usr/local/bin/pgbouncer"),
    ]);
    assert!(settings.enabled);
    assert_eq!(settings.port, 7000);
    assert_eq!(settings.max_db_connections, 30);
    assert_eq!(settings.max_client_conn, 40);
    assert_eq!(settings.binary, "/usr/local/bin/pgbouncer");
}

#[test]
fn pool_env_accepts_minimum_bounds() {
    let settings = pool_settings(&[
        (PORT_ENV, "1"),
        (MAX_DB_CONNECTIONS_ENV, "1"),
        (MAX_CLIENT_CONN_ENV, "1"),
    ]);
    assert_eq!(settings.port, 1);
    assert_eq!(settings.max_db_connections, 1);
    assert_eq!(settings.max_client_conn, 1);
}

#[rstest]
#[case::port_zero(PORT_ENV)]
#[case::db_zero(MAX_DB_CONNECTIONS_ENV)]
#[case::client_zero(MAX_CLIENT_CONN_ENV)]
fn pool_env_rejects_below_minimum(#[case] name: &str) {
    let error = pool_error(&[(name, "0")]);
    assert!(
        matches!(error, PoolSettingsError::TooSmall { .. }),
        "{error:?}"
    );
}

#[test]
fn pool_env_rejects_port_above_maximum() {
    let error = pool_error(&[(PORT_ENV, "65536")]);
    assert!(
        matches!(error, PoolSettingsError::TooLarge { .. }),
        "{error:?}"
    );
}

#[rstest]
#[case::enabled_word(ENABLED_ENV, "maybe")]
#[case::port_word(PORT_ENV, "abc")]
#[case::max_db_word(MAX_DB_CONNECTIONS_ENV, "ten")]
fn pool_env_rejects_invalid_scalars(#[case] name: &str, #[case] value: &str) {
    let error = pool_error(&[(name, value)]);
    assert!(
        matches!(error, PoolSettingsError::Environment(_)),
        "{error:?}"
    );
}

#[test]
fn pool_ignored_options_are_listed() {
    let outcome = pool_environment(&[
        (PORT_ENV, "7000"),
        (MAX_CLIENT_CONN_ENV, "50"),
        (BINARY_ENV, "/usr/local/bin/pgbouncer"),
    ]);
    assert_eq!(
        outcome.ignored,
        vec![PORT_ENV, MAX_CLIENT_CONN_ENV, BINARY_ENV]
    );
    let quiet = pool_environment(&[(ENABLED_ENV, "true")]);
    assert!(quiet.ignored.is_empty());
}

#[test]
fn pool_env_blank_binary_uses_default() {
    let settings = pool_settings(&[(BINARY_ENV, "")]);
    assert_eq!(settings.binary, DEFAULT_BINARY);
}

#[test]
fn pool_limit_caps_only_enabled_writer() {
    let enabled = PgBouncerSettings {
        enabled: true,
        port: DEFAULT_PORT,
        max_db_connections: 20,
        max_client_conn: DEFAULT_MAX_CLIENT_CONN,
        binary: DEFAULT_BINARY.to_owned(),
    };
    assert_eq!(pool_limit(SessionRole::Writer, 50, &enabled), 20);
    assert_eq!(pool_limit(SessionRole::Writer, 5, &enabled), 5);
    assert_eq!(pool_limit(SessionRole::Reader, 50, &enabled), 50);
    assert_eq!(pool_limit(SessionRole::Migration, 50, &enabled), 50);
    let mut disabled = enabled;
    disabled.enabled = false;
    assert_eq!(pool_limit(SessionRole::Writer, 50, &disabled), 50);
}

#[test]
fn native_roots_load_the_system_store() {
    let native = rustls_native_certs::load_native_certs();
    let mut roots = rustls::RootCertStore::empty();
    match add_native_roots(&mut roots) {
        Ok(()) => {
            assert!(native.errors.is_empty());
            assert_eq!(roots.is_empty(), native.certs.is_empty());
            assert!(!roots.is_empty());
        }
        Err(ConnectionOptionsError::NoNativeRoots) => {
            assert!(native.errors.is_empty() && native.certs.is_empty());
        }
        Err(ConnectionOptionsError::NativeRoots { .. }) => assert!(!native.errors.is_empty()),
        Err(other) => panic!("unexpected native trust store error: {other}"),
    }
}

#[rstest]
#[case::disable(
    "postgresql://h/db?sslmode=disable&sslrootcert=ca.pem",
    "postgresql://h/db?sslmode=disable&sslcert=ca.pem&sslaccept=strict"
)]
#[case::prefer(
    "postgresql://h/db?sslmode=prefer&sslrootcert=ca.pem",
    "postgresql://h/db?sslmode=prefer&sslcert=ca.pem&sslaccept=strict"
)]
fn non_verifying_sslmode_pairs_survive_the_projection(#[case] url: &str, #[case] expected: &str) {
    let (translated, _) = translate(url);
    assert_eq!(translated, expected);
}

#[tokio::test]
async fn tls_valid_chain() {
    let server = server::start().await;
    let authority = certificates::authority();
    let ca = write_file(&authority.certificate_pem);
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let url = client_url(
        &server,
        "localhost",
        front.port,
        &[("sslmode", "verify-full"), ("sslrootcert", path_of(&ca))],
    );
    connect_and_query(&url).await;
    assert!(front.handshakes() > 0);
    server.close().await;
}

#[tokio::test]
async fn tls_hostname_mismatch() {
    let server = server::start().await;
    let authority = certificates::authority();
    let ca = write_file(&authority.certificate_pem);
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let url = client_url(
        &server,
        "127.0.0.1",
        front.port,
        &[("sslmode", "verify-full"), ("sslrootcert", path_of(&ca))],
    );
    rejected(&url).await;
    assert!(front.handshakes() > 0);
    server.close().await;
}

#[tokio::test]
async fn tls_unknown_root() {
    let server = server::start().await;
    let authority = certificates::authority();
    let unrelated = write_file(&certificates::unrelated_ca_pem());
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let url = client_url(
        &server,
        "localhost",
        front.port,
        &[
            ("sslmode", "verify-full"),
            ("sslrootcert", path_of(&unrelated)),
        ],
    );
    rejected(&url).await;
    assert!(front.handshakes() > 0);
    server.close().await;
}

fn query_url(params: &[(&str, &str)]) -> String {
    let pairs: Vec<(String, String)> = params
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    format!("postgresql://u@h:5432/db?{}", urlencode_pairs_quote(&pairs))
}

#[test]
fn tls_unreadable_root() {
    let missing = tempfile::TempDir::new().expect("a temporary directory");
    let missing_path = missing.path().join("missing-ca.pem");
    let missing_text = missing_path.to_str().expect("a UTF-8 path");
    let url = query_url(&[("sslmode", "verify-full"), ("sslrootcert", missing_text)]);
    let error = native_connection_options(&url).expect_err("an unreadable root must fail");
    assert!(error.to_string().contains("missing-ca.pem"), "{error}");
}

#[test]
fn tls_malformed_root() {
    let malformed = write_file("this is not a certificate");
    let url = query_url(&[
        ("sslmode", "verify-full"),
        ("sslrootcert", path_of(&malformed)),
    ]);
    let error = native_connection_options(&url).expect_err("a malformed root must fail");
    assert!(error.to_string().contains("CA certificate"), "{error}");
}

#[tokio::test]
async fn tls_verify_ca_checks_hostname() {
    let server = server::start().await;
    let authority = certificates::authority();
    let ca = write_file(&authority.certificate_pem);
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let source = client_url(
        &server,
        "127.0.0.1",
        front.port,
        &[("sslmode", "verify-ca"), ("sslrootcert", path_of(&ca))],
    );
    let (translated, calls) = translate(&source);
    assert!(!calls.is_empty());
    assert!(translated.contains("sslaccept=strict"), "{translated}");
    rejected(&translated).await;
    assert!(front.handshakes() > 0);
    server.close().await;
}

#[tokio::test]
async fn tls_require_without_strict() {
    let server = server::start().await;
    let authority = certificates::self_signed();
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let url = client_url(&server, "localhost", front.port, &[("sslmode", "require")]);
    connect_and_query(&url).await;
    assert!(front.handshakes() > 0);
    server.close().await;
}

#[tokio::test]
async fn tls_prefer_plaintext_fallback() {
    let server = server::start().await;
    let front = Front::start(&server.url, Mode::Plaintext).await;
    let url = client_url(&server, "127.0.0.1", front.port, &[("sslmode", "prefer")]);
    connect_and_query(&url).await;
    assert_eq!(front.handshakes(), 0);
    server.close().await;
}

#[tokio::test]
async fn tls_require_refuses_plaintext_server() {
    let server = server::start().await;
    let front = Front::start(&server.url, Mode::Plaintext).await;
    let url = client_url(&server, "127.0.0.1", front.port, &[("sslmode", "require")]);
    rejected(&url).await;
    server.close().await;
}

#[tokio::test]
async fn tls_blank_and_unknown_sslaccept_verify() {
    let server = server::start().await;
    let authority = certificates::authority();
    let ca = write_file(&authority.certificate_pem);
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let blank = client_url(
        &server,
        "127.0.0.1",
        front.port,
        &[
            ("sslmode", "require"),
            ("sslrootcert", path_of(&ca)),
            ("sslaccept", ""),
        ],
    );
    rejected(&blank).await;
    let unknown = client_url(
        &server,
        "127.0.0.1",
        front.port,
        &[
            ("sslmode", "require"),
            ("sslrootcert", path_of(&ca)),
            ("sslaccept", "unknown"),
        ],
    );
    rejected(&unknown).await;
    assert!(front.handshakes() > 0);
    server.close().await;
}

#[tokio::test]
async fn tls_root_without_mode_verifies() {
    let server = server::start().await;
    let authority = certificates::authority();
    let ca = write_file(&authority.certificate_pem);
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let url = client_url(
        &server,
        "127.0.0.1",
        front.port,
        &[("sslrootcert", path_of(&ca))],
    );
    rejected(&url).await;
    assert!(front.handshakes() > 0);
    server.close().await;
}

#[tokio::test]
async fn tls_require_with_prisma_cert_without_accept_is_unverified() {
    let server = server::start().await;
    let authority = certificates::authority();
    let ca = write_file(&authority.certificate_pem);
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let url = client_url(
        &server,
        "127.0.0.1",
        front.port,
        &[("sslmode", "require"), ("sslcert", path_of(&ca))],
    );
    connect_and_query(&url).await;
    server.close().await;
}

#[tokio::test]
async fn tls_explicit_accept_invalid_certs_stays_unverified() {
    let server = server::start().await;
    let authority = certificates::authority();
    let ca = write_file(&authority.certificate_pem);
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let url = client_url(
        &server,
        "127.0.0.1",
        front.port,
        &[
            ("sslmode", "verify-full"),
            ("sslrootcert", path_of(&ca)),
            ("sslaccept", "accept_invalid_certs"),
        ],
    );
    connect_and_query(&url).await;
    server.close().await;
}

#[tokio::test]
async fn native_last_duplicate_sslaccept_wins() {
    let server = server::start().await;
    let authority = certificates::authority();
    let ca = write_file(&authority.certificate_pem);
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let url = client_url(
        &server,
        "127.0.0.1",
        front.port,
        &[
            ("sslmode", "require"),
            ("sslrootcert", path_of(&ca)),
            ("sslaccept", "accept_invalid_certs"),
            ("sslaccept", "strict"),
        ],
    );
    rejected(&url).await;
    assert!(front.handshakes() > 0);
    server.close().await;
}

fn litellm_probe_files() -> BTreeSet<String> {
    fs::read_dir(env::temp_dir())
        .expect("the temporary directory")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| name.starts_with("litellm-sslcert-"))
        .collect()
}

#[tokio::test]
async fn native_full_ca_bundle_no_probe_file() {
    let server = server::start().await;
    let authority = certificates::authority();
    let bundle = format!(
        "{}\n{}",
        certificates::unrelated_ca_pem(),
        authority.certificate_pem
    );
    let bundle_file = write_file(&bundle);
    let before = litellm_probe_files();
    let front = Front::start(
        &server.url,
        Mode::Tls {
            leaf: authority.leaf.clone(),
            key: authority.key.clone_key(),
        },
    )
    .await;
    let url = client_url(
        &server,
        "localhost",
        front.port,
        &[
            ("sslmode", "verify-full"),
            ("sslrootcert", path_of(&bundle_file)),
        ],
    );
    connect_and_query(&url).await;
    assert_eq!(litellm_probe_files(), before);
    server.close().await;
}
