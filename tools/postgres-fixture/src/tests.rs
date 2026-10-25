#![cfg(test)]

use std::{path::Path, time::Duration};

use testcontainers::core::{IntoContainerPort as _, WaitFor};
use testcontainers::{CopyTargetOptions, GenericImage, ImageExt as _, runners::AsyncRunner as _};
use tokio::time::sleep;
use tokio_postgres::{Client, NoTls};

use crate::connection::{FixtureSource, PostgresFixture, select_source};
use crate::error::FixtureError;
use crate::seed::{DatabaseFixture, FixtureSeed, KeyRow, Membership, TeamRow, UserRow};
use secrecy::{ExposeSecret as _, SecretString};

fn url(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

async fn postgres_server() -> (testcontainers::ContainerAsync<GenericImage>, SecretString) {
    let image = GenericImage::new("postgres", "17")
        .with_wait_for(WaitFor::message_on_stdout("database system is ready"))
        .with_wait_for(WaitFor::seconds(2))
        .with_exposed_port(5432.tcp())
        .with_env_var("POSTGRES_USER", "fixture")
        .with_env_var("POSTGRES_PASSWORD", "fixture")
        .with_env_var("POSTGRES_DB", "postgres");
    let server = image.start().await.unwrap();
    let host = server.get_host().await.unwrap();
    let port = server.get_host_port_ipv4(5432.tcp()).await.unwrap();
    (
        server,
        url(&format!(
            "postgresql://fixture:fixture@{host}:{port}/postgres"
        )),
    )
}

async fn connect_client(connection_url: &SecretString) -> (Client, tokio::task::JoinHandle<()>) {
    let (client, connection) = tokio_postgres::connect(connection_url.expose_secret(), NoTls)
        .await
        .unwrap();
    let driver = tokio::spawn(async move {
        connection.await.unwrap();
    });
    (client, driver)
}

async fn database_exists(admin: &Client, database: &str) -> bool {
    admin
        .query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&database])
        .await
        .unwrap()
        .is_some()
}

fn fixture_seed(user_id: &str, token: &str) -> FixtureSeed {
    FixtureSeed {
        users: vec![UserRow {
            user_id: user_id.to_owned(),
            user_email: None,
        }],
        keys: vec![KeyRow {
            token: SecretString::from(token.to_owned()),
            user_id: user_id.to_owned(),
            team_id: None,
            role: None,
        }],
        teams: Vec::new(),
        memberships: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    }
}

#[test]
fn configured_url_wins() {
    assert!(matches!(
        select_source(Some(&url("postgresql://u:p@localhost/db"))),
        Ok(FixtureSource::Configured(_))
    ));
}

#[test]
fn containers_when_unset() {
    assert!(matches!(
        select_source(None),
        Ok(FixtureSource::Testcontainers)
    ));
}

#[test]
fn empty_configured_url_fails() {
    assert!(matches!(
        select_source(Some(&url(""))),
        Err(FixtureError::EmptyConfiguredUrl)
    ));
    assert!(matches!(
        select_source(Some(&url("not-postgres"))),
        Err(FixtureError::InvalidConfiguredUrl { .. })
    ));
    let FixtureSource::Configured(options) =
        select_source(Some(&url("postgresql://u:secret@localhost/db"))).unwrap()
    else {
        panic!("configured source")
    };
    assert!(!format!("{options:?}").contains("secret"));
}

#[tokio::test]
async fn createdb_privilege_failure() {
    let (_server, admin_url) = postgres_server().await;
    let (admin, _driver) = connect_client(&admin_url).await;
    admin
        .batch_execute("CREATE ROLE fixture_nocreatedb LOGIN PASSWORD 'fixture'")
        .await
        .unwrap();
    let restricted_url = url("postgresql://fixture_nocreatedb:fixture@127.0.0.1/postgres");
    let mut generator = crate::connection::RandomDatabaseName;
    let result = PostgresFixture::provision_with_name_generator(
        Some(&restricted_url),
        &FixtureSeed::default(),
        &mut generator,
    )
    .await;
    assert!(matches!(
        result,
        Err(FixtureError::Database(_)) | Err(FixtureError::SetupCleanup { .. })
    ));
}

#[tokio::test]
async fn isolated_schemas() {
    let (_server, admin_url) = postgres_server().await;
    let (client, _driver) = connect_client(&admin_url).await;
    client.batch_execute("CREATE SCHEMA first_schema; CREATE SCHEMA second_schema; CREATE TABLE first_schema.rows (value TEXT); INSERT INTO first_schema.rows VALUES ('first');").await.unwrap();
    let second_rows = client
        .query("SELECT value FROM second_schema.rows", &[])
        .await;
    assert!(second_rows.is_err());
    let visible = client
        .query("SELECT value FROM first_schema.rows", &[])
        .await
        .unwrap();
    assert_eq!(visible.len(), 1);
}

#[tokio::test]
async fn independent_databases_have_all_baseline_foreign_keys() {
    let seed = FixtureSeed {
        users: vec![UserRow {
            user_id: "user".to_owned(),
            user_email: None,
        }],
        teams: vec![TeamRow {
            team_id: "team".to_owned(),
            team_alias: None,
        }],
        memberships: vec![Membership {
            user_id: "user".to_owned(),
            team_id: "team".to_owned(),
        }],
        keys: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    let (_server, admin_url) = postgres_server().await;
    let mut first_generator = crate::connection::RandomDatabaseName;
    let mut second_generator = crate::connection::RandomDatabaseName;
    let first = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &seed,
        &mut first_generator,
    )
    .await
    .unwrap();
    let second = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &seed,
        &mut second_generator,
    )
    .await
    .unwrap();
    let expected = 14_i64;
    for fixture in [&first, &second] {
        let (client, _driver) = connect_client(&fixture.options.database_url()).await;
        let row = client
            .query_one(
                "SELECT count(*) FROM pg_constraint WHERE contype = 'f'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, i64>(0), expected);
    }
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
}

#[tokio::test]
async fn existing_database_never_dropped() {
    let (_server, admin_url) = postgres_server().await;
    let (admin, _driver) = connect_client(&admin_url).await;
    let configured = url(&format!(
        "{}",
        admin_url.expose_secret().replace("/postgres", "/existing")
    ));
    admin
        .batch_execute("CREATE DATABASE existing")
        .await
        .unwrap();
    let mut generator = crate::connection::RandomDatabaseName;
    let fixture = PostgresFixture::provision_with_name_generator(
        Some(&configured),
        &FixtureSeed::default(),
        &mut generator,
    )
    .await
    .unwrap();
    fixture.close_async().await.unwrap();
    assert!(database_exists(&admin, "existing").await);
}

#[tokio::test]
async fn cleanup_after_failure() {
    let seed = FixtureSeed {
        users: vec![UserRow {
            user_id: "contains\0nul".to_owned(),
            user_email: None,
        }],
        keys: Vec::new(),
        teams: Vec::new(),
        memberships: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    let (_server, admin_url) = postgres_server().await;
    let mut generator = FixedName("failed_fixture");
    let result =
        PostgresFixture::provision_with_name_generator(Some(&admin_url), &seed, &mut generator)
            .await;
    assert!(matches!(result, Err(FixtureError::Database(_))));
    let (admin, _driver) = connect_client(&admin_url).await;
    assert!(!database_exists(&admin, "failed_fixture").await);
}

#[tokio::test]
async fn cleanup_after_cancellation() {
    let (_server, admin_url) = postgres_server().await;
    let (admin, _driver) = connect_client(&admin_url).await;
    let mut generator = FixedName("cancelled_fixture");
    let future = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &FixtureSeed::default(),
        &mut generator,
    );
    while !database_exists(&admin, "cancelled_fixture").await {
        sleep(Duration::from_millis(25)).await;
    }
    drop(future);
    for _ in 0..800 {
        if !database_exists(&admin, "cancelled_fixture").await {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("cancelled fixture database survived cleanup");
}

struct FixedName(&'static str);

impl crate::connection::DatabaseNameGenerator for FixedName {
    fn next_name(&mut self) -> String {
        self.0.to_owned()
    }
}

#[tokio::test]
async fn concurrent_fixture_roles_do_not_leak() {
    let (_server, admin_url) = postgres_server().await;
    let first_seed = fixture_seed("first", "first-token");
    let second_seed = fixture_seed("second", "second-token");
    let first_url = admin_url.clone();
    let second_url = admin_url.clone();
    let (first, second) = tokio::join!(
        tokio::task::spawn_blocking(move || PostgresFixture::provision_from_url(
            Some(&first_url),
            &DatabaseFixture { seed: first_seed }
        )
        .unwrap()),
        tokio::task::spawn_blocking(move || PostgresFixture::provision_from_url(
            Some(&second_url),
            &DatabaseFixture { seed: second_seed }
        )
        .unwrap()),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    let (client, first_driver) = connect_client(&first.options.database_url()).await;
    let row = client
        .query_one("SELECT user_id FROM \"LiteLLM_UserTable\"", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "first");
    drop(client);
    drop(first_driver);
    let (client, second_driver) = connect_client(&second.options.database_url()).await;
    let row = client
        .query_one("SELECT user_id FROM \"LiteLLM_UserTable\"", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "second");
    drop(client);
    drop(second_driver);
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
}

#[test]
fn connection_credentials_redacted() {
    let source = select_source(Some(&url("postgresql://user:secret@localhost/db"))).unwrap();
    assert!(!format!("{source:?}").contains("secret"));
}

#[tokio::test]
async fn unreachable_configured_url_fails() {
    let mut generator = crate::connection::RandomDatabaseName;
    let result = PostgresFixture::provision_with_name_generator(
        Some(&url("postgresql://u:p@127.0.0.1:1/db")),
        &FixtureSeed::default(),
        &mut generator,
    )
    .await;
    result.unwrap_err();
}

#[test]
fn duplicate_identity_fails_before_connection() {
    let seed = FixtureSeed {
        users: vec![
            UserRow {
                user_id: "same".to_owned(),
                user_email: None,
            },
            UserRow {
                user_id: "same".to_owned(),
                user_email: None,
            },
        ],
        keys: Vec::new(),
        teams: Vec::new(),
        memberships: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    assert!(seed.validate().is_err());
}

#[test]
fn duplicate_key_identity_fails_before_connection() {
    let seed = FixtureSeed {
        users: vec![UserRow {
            user_id: "user".to_owned(),
            user_email: None,
        }],
        keys: vec![
            KeyRow {
                token: SecretString::from("same"),
                user_id: "user".to_owned(),
                team_id: None,
                role: None,
            },
            KeyRow {
                token: SecretString::from("same"),
                user_id: "user".to_owned(),
                team_id: None,
                role: None,
            },
        ],
        teams: Vec::new(),
        memberships: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    assert!(seed.validate().is_err());
}

#[test]
fn duplicate_membership_identity_fails_before_connection() {
    let seed = FixtureSeed {
        users: vec![UserRow {
            user_id: "user".to_owned(),
            user_email: None,
        }],
        teams: vec![TeamRow {
            team_id: "team".to_owned(),
            team_alias: None,
        }],
        memberships: vec![
            Membership {
                user_id: "user".to_owned(),
                team_id: "team".to_owned(),
            },
            Membership {
                user_id: "user".to_owned(),
                team_id: "team".to_owned(),
            },
        ],
        keys: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    assert!(seed.validate().is_err());
}

#[tokio::test]
async fn trusted_ca_matching_hostname() {
    let (_server, fixture_url) = tls_postgres().await;
    let connection_url = format!(
        "{fixture_url}?sslmode=verify-full&sslrootcert={}",
        ca_path().display()
    );
    let FixtureSource::Configured(options) = select_source(Some(&url(&connection_url))).unwrap()
    else {
        panic!("configured source")
    };
    assert!(options.uses_tls());
    let mut generator = FixedName("trusted_tls_fixture");
    let fixture = PostgresFixture::provision_with_name_generator(
        Some(&url(&connection_url)),
        &FixtureSeed::default(),
        &mut generator,
    )
    .await
    .unwrap();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn untrusted_ca_rejected() {
    let (_server, fixture_url) = tls_postgres().await;
    let connection_url = format!(
        "{fixture_url}?sslmode=verify-ca&sslrootcert={}",
        server_path().display()
    );
    let FixtureSource::Configured(options) = select_source(Some(&url(&connection_url))).unwrap()
    else {
        panic!("configured source")
    };
    assert!(options.uses_tls());
    let mut generator = FixedName("untrusted_tls_fixture");
    let result = PostgresFixture::provision_with_name_generator(
        Some(&url(&connection_url)),
        &FixtureSeed::default(),
        &mut generator,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn wrong_hostname_rejected() {
    let (_server, fixture_url) = tls_postgres().await;
    let connection_url = format!(
        "{fixture_url}?sslmode=verify-full&sslrootcert={}",
        ca_path().display()
    )
    .replace("localhost", "127.0.0.1");
    let FixtureSource::Configured(options) = select_source(Some(&url(&connection_url))).unwrap()
    else {
        panic!("configured source")
    };
    assert!(options.uses_tls());
    let mut generator = FixedName("wrong_hostname_fixture");
    let result = PostgresFixture::provision_with_name_generator(
        Some(&url(&connection_url)),
        &FixtureSeed::default(),
        &mut generator,
    )
    .await;
    assert!(result.is_err());
}

fn ca_path() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/tls/ca.pem"))
}

fn server_path() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/tls/server.pem"
    ))
}

async fn tls_postgres() -> (testcontainers::ContainerAsync<GenericImage>, String) {
    let image = GenericImage::new("postgres", "17")
        .with_wait_for(WaitFor::message_on_stdout("database system is ready"))
        .with_wait_for(WaitFor::seconds(2))
        .with_exposed_port(5432.tcp())
        .with_entrypoint("/bin/bash")
        .with_env_var("POSTGRES_USER", "fixture")
        .with_env_var("POSTGRES_PASSWORD", "fixture")
        .with_env_var("POSTGRES_DB", "postgres")
        .with_cmd([
            "-c",
            "cp /tls/server.key /var/lib/postgresql/server.key && chown postgres:postgres /var/lib/postgresql/server.key && chmod 600 /var/lib/postgresql/server.key && exec /usr/local/bin/docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tls/server.pem -c ssl_key_file=/var/lib/postgresql/server.key",
        ])
        .with_copy_to(
            CopyTargetOptions::new("/tls/ca.pem").with_mode(0o644),
            ca_path(),
        )
        .with_copy_to(
            CopyTargetOptions::new("/tls/server.pem").with_mode(0o644),
            server_path(),
        )
        .with_copy_to(
            CopyTargetOptions::new("/tls/server.key").with_mode(0o600),
            Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/tls/server.key")),
        );
    let server = image.start().await.unwrap();
    let host = server.get_host().await.unwrap();
    let port = server.get_host_port_ipv4(5432.tcp()).await.unwrap();
    (
        server,
        format!("postgresql://fixture:fixture@{host}:{port}/postgres"),
    )
}
