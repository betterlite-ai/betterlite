#![cfg(test)]

use std::future::{Future as _, poll_fn};
use std::task::Poll;
use std::{process, time::Duration};

use clap::Parser;
use rstest::rstest;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tokio_postgres::{Client, NoTls, types::FromSqlOwned};

use crate::connection::{
    DatabaseNameGenerator, PostgresFixture, RandomDatabaseName, configured_options,
};
use crate::error::FixtureError;
use crate::seed::{CallerRole, FixtureSeed, KeyRow, Membership, TeamRow, UserRow};
use secrecy::{ExposeSecret as _, SecretString};

fn url(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}
#[derive(Parser)]
struct TestEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    postgres_test_url: Option<String>,
}
pub(crate) fn postgres_url() -> SecretString {
    let configured = TestEnvironment::parse_from(["postgres-fixture-tests"]).postgres_test_url;
    url(&configured.expect("POSTGRES_TEST_URL is set, by CI or by nextest's postgres setup script"))
}
pub(crate) async fn connect_client(connection_url: &SecretString) -> (Client, JoinSet<()>) {
    let (client, connection) = tokio_postgres::connect(connection_url.expose_secret(), NoTls)
        .await
        .unwrap();
    let mut driver = JoinSet::new();
    driver.spawn(async move {
        connection.await.unwrap();
    });
    (client, driver)
}
async fn close_driver(client: Client, mut driver: JoinSet<()>) {
    drop(client);
    driver.join_next().await.unwrap().unwrap();
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
    configured_options(Some(&url("postgresql://u:p@localhost/db"))).unwrap();
}
#[test]
fn missing_url_fails() {
    assert!(matches!(
        configured_options(None),
        Err(FixtureError::MissingConfiguredUrl)
    ));
}
#[test]
fn empty_configured_url_fails() {
    assert!(matches!(
        configured_options(Some(&url(""))),
        Err(FixtureError::EmptyConfiguredUrl)
    ));
    assert!(matches!(
        configured_options(Some(&url("not-postgres"))),
        Err(FixtureError::InvalidConfiguredUrl { .. })
    ));
}
#[tokio::test]
async fn createdb_privilege_failure() {
    let admin_url = postgres_url();
    let (admin, _driver) = connect_client(&admin_url).await;
    // CI shares one server across runs and its admin user isn't fixture, so the role is unique, dropped after, and built into the URL
    let role = format!("fixture_nocreatedb_{}", process::id());
    admin
        .batch_execute(&format!(
            "CREATE ROLE {role} LOGIN PASSWORD 'fixture+secret'"
        ))
        .await
        .unwrap();
    let (scheme, address) = admin_url.expose_secret().split_once("://").unwrap();
    let host = address.rsplit_once('@').unwrap().1;
    let restricted_url = url(&format!("{scheme}://{role}:fixture+secret@{host}"));
    let mut generator = RandomDatabaseName;
    let result = PostgresFixture::provision_with_name_generator(
        Some(&restricted_url),
        &FixtureSeed::default(),
        &mut generator,
    )
    .resolve()
    .await;
    admin
        .batch_execute(&format!("DROP ROLE {role}"))
        .await
        .unwrap();
    assert!(matches!(
        result,
        Err(FixtureError::Database(_) | FixtureError::SetupCleanup { .. })
    ));
}
#[tokio::test]
async fn statement_fixtures_are_separate_copies_of_one_schema() {
    let admin_url = postgres_url();
    let statements = vec!["CREATE TABLE rows (value TEXT)".to_owned()];
    let first = PostgresFixture::provision_with_statements(Some(&admin_url), statements.clone())
        .resolve()
        .await
        .unwrap();
    let second = PostgresFixture::provision_with_statements(Some(&admin_url), statements)
        .resolve()
        .await
        .unwrap();
    let (writer, first_driver) = connect_client(&first.options.url).await;
    writer
        .batch_execute("INSERT INTO rows VALUES ('first')")
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
    let first_database = fixture_scalar::<String>(&first, "SELECT current_database()").await;
    let second_database = fixture_scalar::<String>(&second, "SELECT current_database()").await;
    close_driver(writer, first_driver).await;
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
    let (admin, _driver) = connect_client(&admin_url).await;
    let statement = "SELECT 1 FROM pg_database WHERE datname = ANY($1)";
    let names = vec![first_database, second_database];
    let databases = admin.query(statement, &[&names]).await.unwrap();
    assert!(databases.is_empty());
}
#[rstest]
#[case::underscore("verify_full")]
#[case::allow("allow")]
fn rejects_unknown_tls_mode(#[case] value: &str) {
    let configured = url(&format!("postgresql://u:p@localhost/db?sslmode={value}"));
    assert!(
        matches!(configured_options(Some(&configured)), Err(FixtureError::UnknownTlsMode { value: actual }) if actual == value)
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
    let seed = membership_seed();
    let admin_url = postgres_url();
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
    let admin_url = postgres_url();
    let (admin, _driver) = connect_client(&admin_url).await;
    // CI shares one server across runs and names its admin database differently, so the database is unique and dropped after
    let name = format!("existing_{}", process::id());
    let base = admin_url.expose_secret().rsplit_once('/').unwrap().0;
    let configured = url(&format!("{base}/{name}"));
    admin
        .batch_execute(&format!("CREATE DATABASE {name}"))
        .await
        .unwrap();
    let fixture = configured_fixture(&configured, &FixtureSeed::default()).await;
    fixture.close_async().await.unwrap();
    assert!(database_exists(&admin, &name).await);
    let result = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &FixtureSeed::default(),
        &mut FixedName(name.clone()),
    )
    .resolve()
    .await;
    assert!(matches!(result, Err(FixtureError::Database(_))));
    assert!(database_exists(&admin, &name).await);
    admin
        .batch_execute(&format!("DROP DATABASE {name}"))
        .await
        .unwrap();
}
#[tokio::test]
async fn cleanup_after_failure() {
    let mut seed = fixture_seed("contains\0nul", "token");
    seed.keys.clear();
    let admin_url = postgres_url();
    let mut generator = FixedName("failed_fixture".to_owned());
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
    let admin_url = postgres_url();
    let (admin, _driver) = connect_client(&admin_url).await;
    let mut generator = FixedName("cancelled_fixture".to_owned());
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
    let admin_url = postgres_url();
    let (admin, _driver) = connect_client(&admin_url).await;
    let mut generator = FixedName("unclaimed_fixture".to_owned());
    let task = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        &FixtureSeed::default(),
        &mut generator,
    );
    task.close().await.unwrap();
    assert!(!database_exists(&admin, "unclaimed_fixture").await);
}

pub(crate) struct FixedName(pub(crate) String);

impl DatabaseNameGenerator for FixedName {
    fn next_name(&mut self) -> String {
        self.0.clone()
    }
}
#[tokio::test]
async fn concurrent_fixture_roles_do_not_leak() {
    let admin_url = postgres_url();
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
    PostgresFixture::provision_with_name_generator(Some(admin_url), seed, &mut RandomDatabaseName)
        .resolve()
        .await
        .unwrap()
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
    let source = configured_options(Some(&url("postgresql://user:secret@localhost/db"))).unwrap();
    assert!(!format!("{source:?}").contains("secret"));
    let seed = fixture_seed("user", "secret-token");
    assert!(!format!("{seed:?}").contains("secret-token"));
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
