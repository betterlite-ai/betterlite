#![cfg(test)]

mod connect;
mod ensure;
mod fake_postgres;
mod leaf;
mod locks;
mod partition;
mod replica;
mod support;

use std::time::Duration;

use deadpool_postgres::GenericClient as _;

use crate::request_log_indexes::replica_identity::{REPLICA_IDENTITY_FULL_SQL, apply_on};

use support::{
    API_KEY_INDEX, FixedJitter, PostgresTest, RecordingDatabase, RecordingPause, SPEND_LOGS,
    attached_children, create_partition_tree, index_builder, index_dependencies, index_oid,
    index_state_of, informed, install_logs, occurrences, other_schema, replica_identity, role_name,
    run_partition, suffix,
};

#[tokio::test]
async fn postgres_recursive_partition_attachment() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    create_partition_tree(&test).await;
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(run_partition(&builder, builder.schema).await);
    assert_partition_children(&test).await;
    assert_partition_logs(&logs.contents());
    drop_other_schema(&test).await;
    test.finish().await;
}

async fn drop_other_schema(test: &PostgresTest) {
    let other = other_schema(&test.schema);
    test.client
        .batch_execute(&format!("DROP SCHEMA \"{other}\" CASCADE"))
        .await
        .unwrap();
}

async fn assert_partition_children(test: &PostgresTest) {
    assert!(index_state_of(&test.client, API_KEY_INDEX).await.unwrap().1);
    let top = attached_children(&test.client, &test.schema, API_KEY_INDEX).await;
    assert_eq!(
        top,
        [
            format!("LiteLLM_SpendLogs_p1_{}", suffix()),
            format!("LiteLLM_SpendLogs_p2_{}", suffix()),
            format!("LiteLLM_SpendLogs_x_{}", suffix()),
        ]
    );
    let nested_index = format!("LiteLLM_SpendLogs_p1_{}", suffix());
    let nested = attached_children(&test.client, &test.schema, &nested_index).await;
    assert_eq!(
        nested,
        [
            format!("LiteLLM_SpendLogs_p1_a_{}", suffix()),
            format!("LiteLLM_SpendLogs_p1_b_{}", suffix()),
        ]
    );
    let cross_index = format!("LiteLLM_SpendLogs_x_{}", suffix());
    let cross = attached_children(&test.client, &other_schema(&test.schema), &cross_index).await;
    assert_eq!(cross, [format!("LiteLLM_SpendLogs_x_a_{}", suffix())]);
}

fn assert_partition_logs(logs: &str) {
    assert_eq!(occurrences(logs, "Attached index "), 6);
    assert!(informed(
        logs,
        "Attached index LiteLLM_SpendLogs_p2_api_key_startTime_idx on partition LiteLLM_SpendLogs_p2 to LiteLLM_SpendLogs_api_key_startTime_idx"
    ));
}

#[tokio::test]
async fn postgres_partial_progress_survives_failure() {
    let test = PostgresTest::new().await;
    create_blocked_partitions(&test).await;
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(!run_partition(&builder, builder.schema).await);
    let p1_oid = assert_first_child_survived(&test).await;
    let collision = format!("LiteLLM_SpendLogs_p2_{}", suffix());
    test.client
        .batch_execute(&format!("DROP TABLE \"{collision}\""))
        .await
        .unwrap();
    assert!(run_partition(&builder, builder.schema).await);
    assert_rerun_kept_and_attached(&test, p1_oid).await;
    test.finish().await;
}

async fn create_blocked_partitions(test: &PostgresTest) {
    let statement = format!(
        "CREATE TABLE \"{SPEND_LOGS}\" (api_key text, \"startTime\" timestamptz, litellm_call_id text) PARTITION BY RANGE (\"startTime\"); CREATE TABLE \"{SPEND_LOGS}_p1\" PARTITION OF \"{SPEND_LOGS}\" FOR VALUES FROM ('2026-01-01') TO ('2026-02-01'); CREATE TABLE \"{SPEND_LOGS}_p2\" PARTITION OF \"{SPEND_LOGS}\" FOR VALUES FROM ('2026-02-01') TO ('2026-03-01'); CREATE TABLE \"LiteLLM_SpendLogs_p2_{}\" (x int);",
        suffix()
    );
    test.client.batch_execute(&statement).await.unwrap();
}

async fn assert_first_child_survived(test: &PostgresTest) -> u32 {
    let built = format!("LiteLLM_SpendLogs_p1_{}", suffix());
    assert!(index_state_of(&test.client, &built).await.unwrap().1);
    assert_eq!(
        attached_children(&test.client, &test.schema, API_KEY_INDEX).await,
        [built]
    );
    index_oid(&test.client, &format!("LiteLLM_SpendLogs_p1_{}", suffix()))
        .await
        .unwrap()
}

async fn assert_rerun_kept_and_attached(test: &PostgresTest, p1_oid: u32) {
    assert_eq!(
        index_oid(&test.client, &format!("LiteLLM_SpendLogs_p1_{}", suffix()))
            .await
            .unwrap(),
        p1_oid
    );
    let mut expected = vec![
        format!("LiteLLM_SpendLogs_p1_{}", suffix()),
        format!("LiteLLM_SpendLogs_p2_{}", suffix()),
    ];
    expected.sort();
    assert_eq!(
        attached_children(&test.client, &test.schema, API_KEY_INDEX).await,
        expected
    );
}

#[tokio::test]
async fn postgres_replica_busy_table_warns_but_succeeds() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    test.client
        .batch_execute(
            "CREATE TABLE \"LiteLLM_Busy\" (id text); CREATE TABLE \"LiteLLM_Free\" (id text);",
        )
        .await
        .unwrap();
    let (mut busy_client, mut drivers) = test.another_client().await;
    let pin = busy_client.transaction().await.unwrap();
    pin.query("SELECT * FROM \"LiteLLM_Busy\"", &[])
        .await
        .unwrap();
    let database = test.service().await;
    assert!(apply_on(&database, Duration::from_secs(60)).await);
    assert_eq!(replica_identity(&test.client, "LiteLLM_Busy").await, "d");
    assert_eq!(replica_identity(&test.client, "LiteLLM_Free").await, "f");
    assert_replica_busy_logs(&logs.contents());
    pin.rollback().await.unwrap();
    drivers.shutdown().await;
    test.finish().await;
}

fn assert_replica_busy_logs(logs: &str) {
    assert!(informed(
        logs,
        "Applying REPLICA IDENTITY FULL to LiteLLM tables"
    ));
    assert!(informed(
        logs,
        "REPLICA IDENTITY FULL applied to LiteLLM tables"
    ));
    assert!(REPLICA_IDENTITY_FULL_SQL.contains(
        "RAISE WARNING 'REPLICA IDENTITY FULL skipped for %: table busy, retrying next run', target;"
    ));
}

struct Owners {
    owner: String,
    blocked: String,
    viewer: String,
}

#[tokio::test]
async fn postgres_replica_nonlock_error_rolls_back() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    let owners = create_blocked_owners(&test).await;
    let database = test.service().await;
    database
        .client
        .batch_execute(&format!("SET ROLE \"{}\"", owners.viewer))
        .await
        .unwrap();
    assert!(!apply_on(&database, Duration::from_secs(60)).await);
    assert_replica_rollback(&test).await;
    assert_replica_failure_logs(&logs.contents());
    drop(database);
    test.client
        .batch_execute(&format!(
            "DROP OWNED BY \"{}\"; DROP OWNED BY \"{}\"; DROP OWNED BY \"{}\"; REVOKE \"{}\" FROM \"{}\"; DROP ROLE \"{}\"; DROP ROLE \"{}\"; DROP ROLE \"{}\";",
            owners.owner,
            owners.blocked,
            owners.viewer,
            owners.owner,
            owners.viewer,
            owners.viewer,
            owners.owner,
            owners.blocked
        ))
        .await
        .unwrap();
    test.finish().await;
}

async fn create_blocked_owners(test: &PostgresTest) -> Owners {
    let owners = Owners {
        owner: role_name("owner"),
        blocked: role_name("blocked"),
        viewer: role_name("viewer"),
    };
    test.client
        .batch_execute(&format!(
            "CREATE ROLE \"{}\" NOLOGIN;
             CREATE ROLE \"{}\" NOLOGIN;
             CREATE ROLE \"{}\" NOLOGIN;
             GRANT \"{}\" TO \"{}\";
             GRANT USAGE ON SCHEMA \"{}\" TO \"{}\";
             CREATE TABLE \"LiteLLM_OwnedA\" (id text);
             CREATE TABLE \"LiteLLM_OwnedB\" (id text);
             CREATE TABLE \"LiteLLM_Blocked\" (id text);
             ALTER TABLE \"LiteLLM_OwnedA\" OWNER TO \"{}\";
             ALTER TABLE \"LiteLLM_OwnedB\" OWNER TO \"{}\";
             ALTER TABLE \"LiteLLM_Blocked\" OWNER TO \"{}\";",
            owners.owner,
            owners.blocked,
            owners.viewer,
            owners.owner,
            owners.viewer,
            test.schema,
            owners.viewer,
            owners.owner,
            owners.owner,
            owners.blocked
        ))
        .await
        .unwrap();
    owners
}

async fn assert_replica_rollback(test: &PostgresTest) {
    assert_eq!(replica_identity(&test.client, "LiteLLM_OwnedA").await, "d");
    assert_eq!(replica_identity(&test.client, "LiteLLM_OwnedB").await, "d");
    assert_eq!(replica_identity(&test.client, "LiteLLM_Blocked").await, "d");
}

const REPLICA_FAILURE: &str = "Failed to set REPLICA IDENTITY FULL. Logical replication consumers may reject updates to these tables. Grant table ownership to the migration user, or apply `ALTER TABLE ... REPLICA IDENTITY FULL` by hand.";

fn assert_replica_failure_logs(logs: &str) {
    assert!(support::errored(logs, REPLICA_FAILURE));
    assert!(
        logs.contains("must be owner of table"),
        "the database cause is missing from the warning: {logs}"
    );
    assert!(!informed(
        logs,
        "REPLICA IDENTITY FULL applied to LiteLLM tables"
    ));
}
