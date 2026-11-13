#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret as _, SecretString};
use tempfile::TempDir;
use tokio::task::JoinSet;
use tokio_postgres::config::SslMode;
use tokio_postgres::{NoTls, connect};
use url::Url;
use uuid::Uuid;

use super::{TlsMode, tls_connector};
use crate::connection::{FixtureSource, PostgresFixture, select_source};
use crate::error::FixtureError;
use crate::seed::FixtureSeed;
use crate::tests::{FixedName, postgres_server};

mod front;

use front::{Authority, Front, authority};

#[test]
fn missing_sslmode_disables_tls() {
    let configured = SecretString::from("postgresql://u:p@localhost/db");
    let FixtureSource::Configured(options) = select_source(Some(&configured)).unwrap() else {
        panic!("configured source")
    };
    assert_eq!(options.tls_mode, TlsMode::Disable);
    assert_eq!(options.config.get_ssl_mode(), SslMode::Disable);
}

#[test]
fn explicit_sslmode_prefer_is_rejected() {
    let configured = SecretString::from("postgresql://u:p@localhost/db?sslmode=prefer");
    assert!(matches!(
        select_source(Some(&configured)),
        Err(FixtureError::ExplicitPreferTls)
    ));
}

#[tokio::test]
async fn trusted_ca_matching_hostname() {
    let (_server, plain_url) = postgres_server().await;
    let served = authority();
    let directory = TempDir::new().expect("a temporary directory");
    let ca_path = trust_file(&served, &directory);
    let front = Front::start(&plain_url, &served).await;
    let configured = tls_url(&plain_url, "localhost", front.port, &ca_path);
    let database = database_name();
    let mut generator = FixedName(database.clone());
    let fixture = PostgresFixture::provision_with_name_generator(
        Some(&configured),
        &FixtureSeed::default(),
        &mut generator,
    )
    .resolve()
    .await
    .expect("a TLS fixture");
    assert_eq!(fixture_scalar(&fixture).await, 1_i32);
    fixture.close_async().await.expect("the fixture close");
    assert!(front.handshake_count() >= 1);
    assert!(!database_present(&plain_url, &database).await);
}

#[tokio::test]
async fn untrusted_ca_rejected() {
    let (_server, plain_url) = postgres_server().await;
    let served = authority();
    let untrusted = authority();
    let directory = TempDir::new().expect("a temporary directory");
    let ca_path = trust_file(&untrusted, &directory);
    let front = Front::start(&plain_url, &served).await;
    let configured = tls_url(&plain_url, "localhost", front.port, &ca_path);
    let database = database_name();
    let mut generator = FixedName(database.clone());
    let result = PostgresFixture::provision_with_name_generator(
        Some(&configured),
        &FixtureSeed::default(),
        &mut generator,
    )
    .resolve()
    .await;
    assert!(matches!(result, Err(FixtureError::Connect(_))));
    assert!(front.handshake_count() >= 1);
    assert!(!database_present(&plain_url, &database).await);
}

#[tokio::test]
async fn wrong_hostname_rejected() {
    let (_server, plain_url) = postgres_server().await;
    let served = authority();
    let directory = TempDir::new().expect("a temporary directory");
    let ca_path = trust_file(&served, &directory);
    let front = Front::start(&plain_url, &served).await;
    let configured = tls_url(&plain_url, "127.0.0.1", front.port, &ca_path);
    let database = database_name();
    let mut generator = FixedName(database.clone());
    let result = PostgresFixture::provision_with_name_generator(
        Some(&configured),
        &FixtureSeed::default(),
        &mut generator,
    )
    .resolve()
    .await;
    assert!(matches!(result, Err(FixtureError::Connect(_))));
    assert!(front.handshake_count() >= 1);
    assert!(!database_present(&plain_url, &database).await);
}

fn database_name() -> String {
    format!("betterlite_fixture_{}", Uuid::new_v4().simple())
}

fn trust_file(authority: &Authority, directory: &TempDir) -> PathBuf {
    let path = directory.path().join("ca.pem");
    fs::write(&path, &authority.certificate_pem).expect("the trust anchor file");
    path
}

fn tls_url(plain_url: &SecretString, host: &str, port: u16, ca_path: &Path) -> SecretString {
    let mut parsed = Url::parse(plain_url.expose_secret()).expect("the plain server URL");
    parsed.set_host(Some(host)).expect("the TLS host");
    parsed.set_port(Some(port)).expect("the front port");
    let query = format!("sslmode=verify-full&sslrootcert={}", ca_path.display());
    parsed.set_query(Some(&query));
    SecretString::from(parsed.to_string())
}

async fn fixture_scalar(fixture: &PostgresFixture) -> i32 {
    let connector = tls_connector(&fixture.options).expect("a TLS connector");
    let (client, connection) = fixture
        .options
        .config
        .connect(connector)
        .await
        .expect("a TLS connection");
    let mut driver = JoinSet::new();
    driver.spawn(async move {
        connection.await.expect("the connection");
    });
    let row = client.query_one("SELECT 1", &[]).await.expect("SELECT 1");
    drop(client);
    driver
        .join_next()
        .await
        .expect("the driver task")
        .expect("the connection task");
    row.get(0)
}

async fn database_present(plain_url: &SecretString, name: &str) -> bool {
    let (client, connection) = connect(plain_url.expose_secret(), NoTls)
        .await
        .expect("the admin connection");
    let mut driver = JoinSet::new();
    driver.spawn(async move {
        connection.await.expect("the connection");
    });
    let row = client
        .query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&name])
        .await
        .expect("the database lookup");
    drop(client);
    driver
        .join_next()
        .await
        .expect("the driver task")
        .expect("the connection task");
    row.is_some()
}
