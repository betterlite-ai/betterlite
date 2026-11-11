use std::time::{Duration, Instant};

use deadpool_postgres::GenericClient as _;
use secrecy::{ExposeSecret as _, SecretString};

use crate::database_url_policy::query::with_query_value;
use crate::request_log_indexes::replica_identity::{apply_on, apply_replica_identity_full};

use super::fake_postgres::delayed_stalled_server;
use super::support::{
    FakeDatabase, FakeFault, PostgresTest, errored, informed, install_logs, replica_identity,
    role_name, stalled_database_url,
};

async fn create_replica_relations(test: &PostgresTest, other: &str) {
    test.client
        .batch_execute(&format!(
            "CREATE TABLE \"LiteLLM_Plain\" (id text PRIMARY KEY);
             CREATE TABLE \"LiteLLM_Partitioned\" (id text, day date) PARTITION BY RANGE (day);
             CREATE TABLE \"LiteLLM_Partitioned_a\" PARTITION OF \"LiteLLM_Partitioned\" FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');
             CREATE VIEW \"LiteLLM_View\" AS SELECT 1 AS one;
             CREATE TABLE \"Other_Table\" (id text);
             CREATE TABLE litellm_lower (id text);
             CREATE SCHEMA \"{other}\";
             CREATE TABLE \"{other}\".\"LiteLLM_Foreign\" (id text);"
        ))
        .await
        .unwrap();
}

async fn assert_replica_selection(test: &PostgresTest, other: &str) {
    assert_eq!(replica_identity(&test.client, "LiteLLM_Plain").await, "f");
    assert_eq!(
        replica_identity(&test.client, "LiteLLM_Partitioned_a").await,
        "f"
    );
    assert_ne!(
        replica_identity(&test.client, "LiteLLM_Partitioned").await,
        "f"
    );
    assert_eq!(replica_identity(&test.client, "Other_Table").await, "d");
    assert_eq!(replica_identity(&test.client, "litellm_lower").await, "d");
    let view: String = test
        .client
        .query_one("SELECT relreplident::text FROM pg_class WHERE relname = 'LiteLLM_View' AND relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema())", &[])
        .await
        .unwrap()
        .get(0);
    assert_ne!(view, "f");
    let foreign: String = test
        .client
        .query_one("SELECT relreplident::text FROM pg_class WHERE relname = 'LiteLLM_Foreign' AND relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = $1)", &[&other])
        .await
        .unwrap()
        .get(0);
    assert_eq!(foreign, "d");
}

#[tokio::test]
async fn replica_identity_selects_plain_visible_litellm_tables() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    let other = format!("{}x", test.schema);
    create_replica_relations(&test, &other).await;
    let database = test.service().await;
    assert!(apply_on(&database, Duration::from_secs(60)).await);
    assert_replica_selection(&test, &other).await;
    let captured = logs.contents();
    assert!(informed(
        &captured,
        "Applying REPLICA IDENTITY FULL to LiteLLM tables"
    ));
    assert!(informed(
        &captured,
        "REPLICA IDENTITY FULL applied to LiteLLM tables"
    ));
    test.client
        .batch_execute(&format!("DROP SCHEMA \"{other}\" CASCADE"))
        .await
        .unwrap();
    test.finish().await;
}

#[tokio::test]
async fn apply_replica_identity_full_applies_on_the_database() {
    let test = PostgresTest::new().await;
    test.client
        .batch_execute("CREATE TABLE \"LiteLLM_Applied\" (id text PRIMARY KEY);")
        .await
        .unwrap();
    assert!(apply_replica_identity_full(&test.url, Duration::from_secs(60)).await);
    assert_eq!(replica_identity(&test.client, "LiteLLM_Applied").await, "f");
    test.finish().await;
}

#[tokio::test]
async fn replica_identity_uses_the_url_schema_parameter() {
    let test = PostgresTest::new().await;
    test.client
        .batch_execute("CREATE TABLE \"LiteLLM_Prisma\" (id text PRIMARY KEY);")
        .await
        .unwrap();
    let plain = test.fixture.options.url.clone();
    assert!(apply_replica_identity_full(&plain, Duration::from_secs(60)).await);
    assert_eq!(replica_identity(&test.client, "LiteLLM_Prisma").await, "d");
    let url = SecretString::from(
        with_query_value(plain.expose_secret(), "schema", &test.schema).unwrap(),
    );
    assert!(apply_replica_identity_full(&url, Duration::from_secs(60)).await);
    assert_eq!(replica_identity(&test.client, "LiteLLM_Prisma").await, "f");
    test.finish().await;
}

#[tokio::test]
async fn replica_identity_already_full_unchanged() {
    let test = PostgresTest::new().await;
    let owner = role_name("owner");
    let viewer = role_name("viewer");
    test.client
        .batch_execute(&format!(
            "CREATE ROLE \"{owner}\" NOLOGIN;
             CREATE ROLE \"{viewer}\" NOLOGIN;
             GRANT USAGE ON SCHEMA \"{}\" TO \"{viewer}\";
             CREATE TABLE \"LiteLLM_Full\" (id text PRIMARY KEY);
             ALTER TABLE \"LiteLLM_Full\" OWNER TO \"{owner}\";
             ALTER TABLE \"LiteLLM_Full\" REPLICA IDENTITY FULL;",
            test.schema
        ))
        .await
        .unwrap();
    let database = test.service().await;
    database
        .client
        .batch_execute(&format!("SET ROLE \"{viewer}\""))
        .await
        .unwrap();
    assert!(apply_on(&database, Duration::from_secs(60)).await);
    assert_eq!(replica_identity(&test.client, "LiteLLM_Full").await, "f");
    drop(database);
    test.client
        .batch_execute(&format!(
            "DROP TABLE \"LiteLLM_Full\"; REVOKE USAGE ON SCHEMA \"{}\" FROM \"{viewer}\"; DROP ROLE \"{owner}\"; DROP ROLE \"{viewer}\";",
            test.schema
        ))
        .await
        .unwrap();
    test.finish().await;
}

#[tokio::test(start_paused = true)]
async fn replica_deadline_returns_false() {
    let (logs, _guard) = install_logs();
    let fake = FakeDatabase::default();
    fake.configure(|state| {
        state
            .statement_faults
            .push(("DO $$".to_owned(), FakeFault::Pending));
    });
    assert!(!apply_on(&fake, Duration::from_millis(100)).await);
    let captured = logs.contents();
    assert!(errored(
        &captured,
        "Timed out setting REPLICA IDENTITY FULL on LiteLLM tables"
    ));
    assert!(!informed(
        &captured,
        "REPLICA IDENTITY FULL applied to LiteLLM tables"
    ));
}

#[tokio::test]
async fn replica_connection_failure_warns_and_returns_false() {
    let (logs, _guard) = install_logs();
    let url = SecretString::from(
        "postgresql://betterlite:betterlite@127.0.0.1:1/betterlite?sslmode=disable",
    );
    assert!(!apply_replica_identity_full(&url, Duration::from_secs(60)).await);
    let captured = logs.contents();
    assert!(informed(
        &captured,
        "Applying REPLICA IDENTITY FULL to LiteLLM tables"
    ));
    assert!(errored(
        &captured,
        "Could not run the REPLICA IDENTITY FULL statement. Logical replication consumers may reject updates to these tables."
    ));
    assert!(!informed(
        &captured,
        "REPLICA IDENTITY FULL applied to LiteLLM tables"
    ));
}

#[tokio::test]
async fn replica_identity_connect_times_out() {
    let (logs, _guard) = install_logs();
    let (url, mut sockets) = stalled_database_url().await;
    assert!(!apply_replica_identity_full(&url, Duration::from_millis(200)).await);
    let captured = logs.contents();
    assert!(informed(
        &captured,
        "Applying REPLICA IDENTITY FULL to LiteLLM tables"
    ));
    assert!(errored(
        &captured,
        "Timed out setting REPLICA IDENTITY FULL on LiteLLM tables"
    ));
    assert!(!informed(
        &captured,
        "REPLICA IDENTITY FULL applied to LiteLLM tables"
    ));
    sockets.shutdown().await;
}

#[tokio::test]
async fn replica_connect_time_counts_against_the_command_budget() {
    let (logs, _guard) = install_logs();
    let (url, mut servers) = delayed_stalled_server(Duration::from_millis(800)).await;
    let budget = Duration::from_secs(1);
    let started = Instant::now();
    assert!(!apply_replica_identity_full(&url, budget).await);
    let elapsed = started.elapsed();
    assert!(
        elapsed >= budget,
        "the run ended before its command budget: {elapsed:?}"
    );
    assert!(
        elapsed < budget + Duration::from_millis(400),
        "the connect delay was not charged against the command budget: {elapsed:?}"
    );
    assert!(errored(
        &logs.contents(),
        "Timed out setting REPLICA IDENTITY FULL on LiteLLM tables"
    ));
    servers.shutdown().await;
}

#[tokio::test]
async fn native_replica_execution_needs_no_prisma() {
    let (logs, _guard) = install_logs();
    let fake = FakeDatabase::default();
    assert!(apply_on(&fake, Duration::from_secs(60)).await);
    let statements = fake.state().statements.clone();
    assert_eq!(statements.len(), 1);
    assert!(statements[0].trim_start().starts_with("DO $$"));
    assert!(statements[0].contains("ALTER TABLE %s REPLICA IDENTITY FULL"));
    assert!(statements[0].contains(r"c.relname LIKE 'LiteLLM\_%'"));
    assert!(statements[0].contains("SET LOCAL lock_timeout = '5s'"));
    let captured = logs.contents();
    let begin = captured
        .find("Applying REPLICA IDENTITY FULL to LiteLLM tables")
        .unwrap();
    let done = captured
        .find("REPLICA IDENTITY FULL applied to LiteLLM tables")
        .unwrap();
    assert!(begin < done);
}
