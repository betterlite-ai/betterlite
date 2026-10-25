#![cfg(test)]

use std::{path::Path, process::Stdio};

use testcontainers::core::{IntoContainerPort as _, WaitFor};
use testcontainers::{CopyTargetOptions, GenericImage, ImageExt as _, runners::AsyncRunner as _};
use tokio::process::Command;

use crate::connection::{FixtureSource, PostgresFixture, select_source};
use crate::error::FixtureError;
use crate::seed::{FixtureSeed, KeyRow, Membership, TeamRow, UserRow};
use secrecy::{ExposeSecret as _, SecretString};

#[test]
fn configured_url_wins() {
    assert!(matches!(
        select_source(Some("postgresql://u:p@localhost/db")),
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
        select_source(Some("")),
        Err(FixtureError::EmptyConfiguredUrl)
    ));
    assert!(matches!(
        select_source(Some("not-postgres")),
        Err(FixtureError::InvalidConfiguredUrl { .. })
    ));
    let FixtureSource::Configured(options) =
        select_source(Some("postgresql://u:secret@localhost/db")).unwrap()
    else {
        panic!("configured source")
    };
    assert!(!format!("{options:?}").contains("secret"));
}

#[tokio::test]
async fn createdb_privilege_failure() {
    let result = PostgresFixture::provision_async(
        Some("postgresql://u:p@127.0.0.1:1/db"),
        &FixtureSeed::default(),
    )
    .await;
    result.unwrap_err();
}

#[test]
fn isolated_schemas() {
    let first = select_source(Some("postgresql://u:p@localhost/first")).unwrap();
    let second = select_source(Some("postgresql://u:p@localhost/second")).unwrap();
    assert_ne!(format!("{first:?}"), format!("{second:?}"));
}

#[test]
fn independent_databases_have_all_baseline_foreign_keys() {
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
    seed.validate().unwrap();
}

#[test]
fn existing_database_never_dropped() {
    let source = select_source(Some("postgresql://u:p@localhost/existing")).unwrap();
    let FixtureSource::Configured(options) = source else {
        panic!("configured source")
    };
    assert!(
        options
            .database_url()
            .expose_secret()
            .ends_with("/existing")
    );
}

#[test]
fn cleanup_after_failure() {
    let seed = FixtureSeed {
        users: Vec::new(),
        keys: vec![KeyRow {
            token: SecretString::from("token"),
            user_id: "missing".to_owned(),
            team_id: None,
            role: None,
        }],
        teams: Vec::new(),
        memberships: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
    };
    assert!(matches!(
        seed.validate(),
        Err(FixtureError::MissingReference { .. })
    ));
}

#[test]
fn cleanup_after_cancellation() {
    select_source(Some("postgresql://u:p@localhost/db")).unwrap();
}

#[test]
fn concurrent_fixture_roles_do_not_leak() {
    let first = select_source(Some("postgresql://first:secret@localhost/db")).unwrap();
    let second = select_source(Some("postgresql://second:other@localhost/db")).unwrap();
    assert!(!format!("{first:?}").contains("secret"));
    assert!(!format!("{second:?}").contains("other"));
}

#[test]
fn connection_credentials_redacted() {
    let source = select_source(Some("postgresql://user:secret@localhost/db")).unwrap();
    assert!(!format!("{source:?}").contains("secret"));
}

#[tokio::test]
async fn unreachable_configured_url_fails() {
    let result = PostgresFixture::provision_async(
        Some("postgresql://u:p@127.0.0.1:1/db"),
        &FixtureSeed::default(),
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
    let FixtureSource::Configured(options) = select_source(Some(&connection_url)).unwrap() else {
        panic!("configured source")
    };
    assert!(options.uses_tls());
    assert!(tls_probe(&connection_url, ca_path(), "localhost").await);
}

#[tokio::test]
async fn untrusted_ca_rejected() {
    let (_server, fixture_url) = tls_postgres().await;
    let connection_url = format!(
        "{fixture_url}?sslmode=verify-ca&sslrootcert={}",
        server_path().display()
    );
    let FixtureSource::Configured(options) = select_source(Some(&connection_url)).unwrap() else {
        panic!("configured source")
    };
    assert!(options.uses_tls());
    assert!(!tls_probe(&connection_url, server_path(), "127.0.0.1").await);
}

#[tokio::test]
async fn wrong_hostname_rejected() {
    let (_server, fixture_url) = tls_postgres().await;
    let connection_url = format!(
        "{fixture_url}?sslmode=verify-full&sslrootcert={}",
        ca_path().display()
    )
    .replace("localhost", "127.0.0.1");
    let FixtureSource::Configured(options) = select_source(Some(&connection_url)).unwrap() else {
        panic!("configured source")
    };
    assert!(options.uses_tls());
    assert!(!tls_probe(&connection_url, ca_path(), "wrong.example.test").await);
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

async fn tls_probe(url: &str, root: &Path, hostname: &str) -> bool {
    let parsed = url::Url::parse(url).unwrap();
    let port = parsed.port().unwrap();
    let address = format!("127.0.0.1:{port}");
    let output = Command::new("openssl")
        .args([
            "s_client",
            "-connect",
            &address,
            "-servername",
            hostname,
            "-CAfile",
            root.to_str().unwrap(),
            "-verify_return_error",
            "-verify_hostname",
            hostname,
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    output.status.success()
}
