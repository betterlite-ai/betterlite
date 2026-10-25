#![cfg(test)]

use std::future::{Future as _, poll_fn};
use std::task::Poll;
use std::{env::split_paths, ffi::OsString, path::Path, time::Duration};

use clap::Parser;
use rstest::rstest;
use testcontainers::core::{IntoContainerPort as _, WaitFor};
use testcontainers::{
    ContainerRequest, CopyTargetOptions, GenericImage, ImageExt as _, runners::AsyncRunner as _,
};
use tokio::process::Command;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tokio_postgres::{Client, NoTls, types::FromSqlOwned};

use crate::connection::{
    DatabaseNameGenerator, FixtureSource, PostgresFixture, RandomDatabaseName, select_source,
};
use crate::error::FixtureError;
use crate::seed::{CallerRole, FixtureSeed, KeyRow, Membership, TeamRow, UserRow};
use secrecy::{ExposeSecret as _, SecretString};

fn url(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}
fn postgres_image(image: GenericImage) -> ContainerRequest<GenericImage> {
    image
        .with_wait_for(WaitFor::message_on_stdout("database system is ready"))
        .with_wait_for(WaitFor::seconds(2))
        .with_exposed_port(5432.tcp())
        .with_env_var("POSTGRES_USER", "fixture")
        .with_env_var("POSTGRES_PASSWORD", "fixture+secret")
        .with_env_var("POSTGRES_DB", "postgres")
}
async fn postgres_server() -> (
    Option<testcontainers::ContainerAsync<GenericImage>>,
    SecretString,
) {
    if let Some(configured_url) = test_environment().postgres_test_url {
        return (None, url(&configured_url));
    }
    let server = postgres_image(GenericImage::new("postgres", "17"))
        .start()
        .await
        .unwrap();
    let connection_url = server_url(&server).await;
    (Some(server), url(&connection_url))
}
#[derive(Parser)]
struct TestEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    postgres_test_url: Option<String>,
    #[arg(long, env = "PATH")]
    path: Option<OsString>,
}
fn test_environment() -> TestEnvironment {
    TestEnvironment::parse_from(["postgres-fixture-tests"])
}
// CI has no docker binary, so configured-URL tests must not try to start one.
fn docker_available() -> bool {
    let Some(path) = test_environment().path else {
        return false;
    };
    split_paths(Path::new(&path).as_os_str()).any(|directory| directory.join("docker").is_file())
}
macro_rules! skip_without_postgres {
    () => {
        if !(test_environment().postgres_test_url.is_some() || docker_available()) {
            return;
        }
    };
}
macro_rules! skip_without_docker {
    () => {
        if !docker_available() {
            return;
        }
    };
}
async fn server_url(server: &testcontainers::ContainerAsync<GenericImage>) -> String {
    let host = server.get_host().await.unwrap();
    let port = server.get_host_port_ipv4(5432.tcp()).await.unwrap();
    format!("postgresql://fixture:fixture+secret@{host}:{port}/postgres")
}
async fn connect_client(connection_url: &SecretString) -> (Client, JoinSet<()>) {
    let (client, connection) = tokio_postgres::connect(connection_url.expose_secret(), NoTls)
        .await
        .unwrap();
    let mut driver = JoinSet::new();
    driver.spawn(async move {
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
    let mut seed = FixtureSeed::default();
    seed.users.push(UserRow {
        user_id: user_id.to_owned(),
        user_email: None,
    });
    seed.keys.push(KeyRow {
        token: url(token),
        user_id: user_id.to_owned(),
        team_id: None,
        role: None,
    });
    seed
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
}
#[tokio::test]
async fn createdb_privilege_failure() {
    skip_without_postgres!();
    let (_server, admin_url) = postgres_server().await;
    let (admin, _driver) = connect_client(&admin_url).await;
    admin
        .batch_execute("CREATE ROLE fixture_nocreatedb LOGIN PASSWORD 'fixture+secret'")
        .await
        .unwrap();
    let restricted_url = url(&admin_url
        .expose_secret()
        .replace("//fixture:", "//fixture_nocreatedb:"));
    let mut generator = RandomDatabaseName;
    let result = PostgresFixture::provision_with_name_generator(
        Some(&restricted_url),
        &FixtureSeed::default(),
        &mut generator,
    )
    .resolve()
    .await;
    assert!(matches!(
        result,
        Err(FixtureError::Database(_) | FixtureError::SetupCleanup { .. })
    ));
}
#[tokio::test]
async fn isolated_schemas() {
    skip_without_postgres!();
    let (_server, admin_url) = postgres_server().await;
    let first = PostgresFixture::provision_schema(Some(&admin_url))
        .resolve()
        .await
        .unwrap();
    let second = PostgresFixture::provision_schema(Some(&admin_url))
        .resolve()
        .await
        .unwrap();
    let (writer, mut first_driver) = connect_client(&first.options.url).await;
    let (reader, mut second_driver) = connect_client(&second.options.url).await;
    writer
        .batch_execute("CREATE TABLE rows (value TEXT); INSERT INTO rows VALUES ('first')")
        .await
        .unwrap();
    reader
        .batch_execute("CREATE TABLE rows (value TEXT)")
        .await
        .unwrap();
    assert_eq!(
        fixture_scalar::<String>(&first, "SELECT value FROM rows").await,
        "first"
    );
    assert_eq!(
        fixture_scalar::<i64>(&second, "SELECT count(*) FROM rows").await,
        0
    );
    drop(writer);
    drop(reader);
    first_driver.join_next().await.unwrap().unwrap();
    second_driver.join_next().await.unwrap().unwrap();
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
    let (admin, _driver) = connect_client(&admin_url).await;
    let statement = "SELECT 1 FROM pg_namespace WHERE nspname LIKE 'betterlite_fixture_%'";
    let schemas = admin.query(statement, &[]).await.unwrap();
    assert!(schemas.is_empty());
}
#[tokio::test]
async fn container_removed_on_drop() {
    skip_without_docker!();
    let fixture = PostgresFixture::provision_with_name_generator(
        None,
        &FixtureSeed::default(),
        &mut RandomDatabaseName,
    )
    .resolve()
    .await
    .unwrap();
    let id = fixture.container.as_ref().unwrap().id().to_owned();
    assert!(docker_containers(&id).await.contains(&id));
    drop(fixture);
    timeout(Duration::from_secs(10), async {
        while docker_containers(&id).await.contains(&id) {
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert!(docker_containers(&id).await.is_empty());
}

async fn docker_containers(id: &str) -> String {
    let output = timeout(
        Duration::from_secs(5),
        Command::new("docker")
            .args([
                "ps",
                "--all",
                "--quiet",
                "--no-trunc",
                "--filter",
                &format!("id={id}"),
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
#[rstest]
#[case::underscore("verify_full")]
#[case::allow("allow")]
fn rejects_unknown_tls_mode(#[case] value: &str) {
    let configured = url(&format!("postgresql://u:p@localhost/db?sslmode={value}"));
    assert!(
        matches!(select_source(Some(&configured)), Err(FixtureError::UnknownTlsMode { value: actual }) if actual == value)
    );
}
#[rstest]
#[case::conflicting(CallerRole::ProxyAdmin)]
#[case::matching(CallerRole::InternalUser)]
fn validates_user_roles(#[case] second_role: CallerRole) {
    let mut seed = fixture_seed("user", "first-token");
    seed.keys.first_mut().unwrap().role = Some(CallerRole::InternalUser);
    seed.keys.push(KeyRow {
        token: url("second-token"),
        user_id: "user".to_owned(),
        team_id: None,
        role: Some(second_role),
    });
    let result = seed.validate();
    if second_role == CallerRole::InternalUser {
        result.unwrap();
    } else {
        assert!(
            matches!(result, Err(FixtureError::ConflictingUserRole { user_id }) if user_id == "user")
        );
    }
}
#[tokio::test]
async fn independent_databases_have_all_baseline_foreign_keys() {
    skip_without_postgres!();
    let seed = membership_seed();
    let (_server, admin_url) = postgres_server().await;
    let first = configured_fixture(&admin_url, &seed).await;
    let second = configured_fixture(&admin_url, &seed).await;
    let statement = "SELECT count(*) FROM pg_constraint WHERE contype = 'f'";
    assert_eq!(fixture_scalar::<i64>(&first, statement).await, 14);
    assert_eq!(fixture_scalar::<i64>(&second, statement).await, 14);
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
}
#[tokio::test]
async fn existing_database_never_dropped() {
    skip_without_postgres!();
    let (_server, admin_url) = postgres_server().await;
    let (admin, _driver) = connect_client(&admin_url).await;
    let configured = url(&admin_url.expose_secret().replace("/postgres", "/existing"));
    admin
        .batch_execute("CREATE DATABASE existing")
        .await
        .unwrap();
    let fixture = configured_fixture(&configured, &FixtureSeed::default()).await;
    fixture.close_async().await.unwrap();
    assert!(database_exists(&admin, "existing").await);
    let result = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &FixtureSeed::default(),
        &mut FixedName("existing"),
    )
    .resolve()
    .await;
    assert!(matches!(result, Err(FixtureError::Database(_))));
    assert!(database_exists(&admin, "existing").await);
}
#[tokio::test]
async fn cleanup_after_failure() {
    skip_without_postgres!();
    let mut seed = fixture_seed("contains\0nul", "token");
    seed.keys.clear();
    let (_server, admin_url) = postgres_server().await;
    let mut generator = FixedName("failed_fixture");
    let result =
        PostgresFixture::provision_with_name_generator(Some(&admin_url), &seed, &mut generator)
            .resolve()
            .await;
    assert!(matches!(result, Err(FixtureError::Database(_))));
    let (admin, _driver) = connect_client(&admin_url).await;
    assert!(!database_exists(&admin, "failed_fixture").await);
}
#[tokio::test]
async fn cleanup_after_cancellation() {
    skip_without_postgres!();
    let (_server, admin_url) = postgres_server().await;
    let (admin, _driver) = connect_client(&admin_url).await;
    let mut generator = FixedName("cancelled_fixture");
    let mut task = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &FixtureSeed::default(),
        &mut generator,
    );
    let mut future = Box::pin(task.resolve());
    poll_fn(|context| {
        assert!(future.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    timeout(Duration::from_secs(20), async {
        while !database_exists(&admin, "cancelled_fixture").await {
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    drop(future);
    task.close().await.unwrap();
    assert!(!database_exists(&admin, "cancelled_fixture").await);
}
#[tokio::test]
async fn cleanup_after_unclaimed_result() {
    skip_without_postgres!();
    let (_server, admin_url) = postgres_server().await;
    let (admin, _driver) = connect_client(&admin_url).await;
    let mut generator = FixedName("unclaimed_fixture");
    let task = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &FixtureSeed::default(),
        &mut generator,
    );
    task.close().await.unwrap();
    assert!(!database_exists(&admin, "unclaimed_fixture").await);
}

struct FixedName(&'static str);

impl DatabaseNameGenerator for FixedName {
    fn next_name(&mut self) -> String {
        self.0.to_owned()
    }
}
#[tokio::test]
async fn concurrent_fixture_roles_do_not_leak() {
    skip_without_postgres!();
    let (_server, admin_url) = postgres_server().await;
    let mut first_task = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &fixture_seed("first", "first-token"),
        &mut RandomDatabaseName,
    );
    let mut second_task = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &fixture_seed("second", "second-token"),
        &mut RandomDatabaseName,
    );
    let first = first_task.resolve().await.unwrap();
    let second = second_task.resolve().await.unwrap();
    let statement = "SELECT user_id FROM \"LiteLLM_UserTable\"";
    assert_eq!(fixture_scalar::<String>(&first, statement).await, "first");
    assert_eq!(fixture_scalar::<String>(&second, statement).await, "second");
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
}
async fn configured_fixture(admin_url: &SecretString, seed: &FixtureSeed) -> PostgresFixture {
    let fixture = PostgresFixture::provision_with_name_generator(
        Some(admin_url),
        seed,
        &mut RandomDatabaseName,
    )
    .resolve()
    .await
    .unwrap();
    assert!(fixture.container.is_none());
    fixture
}
async fn fixture_scalar<T>(fixture: &PostgresFixture, statement: &str) -> T
where
    T: FromSqlOwned,
{
    let (client, mut driver) = connect_client(&fixture.options.url).await;
    let row = client.query_one(statement, &[]).await.unwrap();
    drop(client);
    driver.join_next().await.unwrap().unwrap();
    row.get(0)
}
fn membership_seed() -> FixtureSeed {
    let mut seed = fixture_seed("user", "token");
    seed.teams.push(TeamRow {
        team_id: "team".to_owned(),
        team_alias: None,
    });
    seed.memberships.push(Membership {
        user_id: "user".to_owned(),
        team_id: "team".to_owned(),
    });
    seed
}
#[test]
fn connection_credentials_redacted() {
    let source = select_source(Some(&url("postgresql://user:secret@localhost/db"))).unwrap();
    assert!(!format!("{source:?}").contains("secret"));
}

#[tokio::test]
async fn unreachable_configured_url_fails() {
    let mut generator = RandomDatabaseName;
    let result = PostgresFixture::provision_with_name_generator(
        Some(&url("postgresql://u:p@127.0.0.1:1/db")),
        &FixtureSeed::default(),
        &mut generator,
    )
    .resolve()
    .await;
    assert!(matches!(result, Err(FixtureError::Connect(_))));
}

#[rstest]
#[case::user("user")]
#[case::key("key")]
#[case::membership("membership")]
fn duplicate_identity_fails_before_connection(#[case] kind: &str) {
    let mut seed = membership_seed();
    match kind {
        "user" => seed.users.extend_from_within(..),
        "key" => seed.keys.extend_from_within(..),
        "membership" => seed.memberships.extend_from_within(..),
        _ => panic!("unknown identity"),
    }
    assert!(matches!(
        seed.validate(),
        Err(FixtureError::DuplicateIdentity { .. })
    ));
}

#[rstest]
#[case::trusted_ca_matching_hostname(ca_path(), "localhost", true)]
#[case::untrusted_ca_rejected(server_path(), "localhost", false)]
#[case::wrong_hostname_rejected(ca_path(), "127.0.0.1", false)]
#[tokio::test]
async fn verifies_tls_server(
    #[case] certificate: &Path,
    #[case] hostname: &str,
    #[case] trusted: bool,
) {
    skip_without_docker!();
    let (_server, fixture_url) = tls_postgres().await;
    let connection_url = format!(
        "{fixture_url}?sslmode=verify-full&sslrootcert={}",
        certificate.display()
    )
    .replace("localhost", hostname);
    let result = PostgresFixture::provision_with_name_generator(
        Some(&url(&connection_url)),
        &FixtureSeed::default(),
        &mut RandomDatabaseName,
    )
    .resolve()
    .await;
    if trusted {
        result.unwrap().close_async().await.unwrap();
    } else {
        assert!(matches!(result, Err(FixtureError::Connect(_))));
    }
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
    let image = postgres_image(GenericImage::new("postgres", "17").with_entrypoint("/bin/bash"))
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
    let connection_url = server_url(&server).await;
    (server, connection_url)
}
