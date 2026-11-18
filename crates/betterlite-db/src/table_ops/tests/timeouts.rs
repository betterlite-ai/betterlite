#![cfg(test)]
use crate::table_ops::reads::configs::ConfigReads as _;
use crate::table_ops::reads::users::UserReads as _;
use crate::table_ops::updates::batches::BatchUpdates as _;

use std::collections::BTreeMap;
use std::time::Duration;

use super::support::{Harness, Ports, TableClient, wait_for_idle};
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::table_ops::{
    DEFAULT_ACQUIRE_TIMEOUT, DEFAULT_QUERY_TIMEOUT, DEFAULT_TEAM_LIST_LIMIT, TableOpsError,
    TableSettings,
};

#[tokio::test]
async fn a_read_waiting_on_a_lock_times_out_and_names_the_deadline() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.batch_execute("BEGIN; LOCK TABLE \"LiteLLM_Config\" IN ACCESS EXCLUSIVE MODE")
        .await
        .expect("the other connection holds the lock");
    let limited = harness.pool_with_size(1);
    let client = TableClient::new(limited.clone());
    let ports = Ports::new();
    let ops = ports.operations_with(
        &client,
        TableSettings {
            team_list_limit: DEFAULT_TEAM_LIST_LIMIT,
            acquire_timeout: DEFAULT_ACQUIRE_TIMEOUT,
            query_timeout: Duration::from_millis(100),
        },
    );
    let error = ops
        .configs_named(&[String::from("settings")])
        .await
        .expect_err("the locked table read cannot finish");
    assert!(matches!(
        error,
        TableOpsError::Timeout {
            operation: "query",
            deadline,
        } if deadline == Duration::from_millis(100)
    ));
    assert_eq!(
        limited.status().size,
        0,
        "a timed-out connection is discarded"
    );
    let fresh = limited.get().await.unwrap();
    wait_for_idle(&fresh, "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock'").await;
    drop(fresh);
    db.batch_execute("ROLLBACK")
        .await
        .expect("the lock is released");
    harness.close().await;
}

#[tokio::test]
async fn a_batch_statement_waiting_on_a_lock_times_out() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.batch_execute(
        "INSERT INTO \"LiteLLM_VerificationToken\" (token, key_alias, models) VALUES ('tok-1', 'original', ARRAY[]::text[])",
    )
    .await
    .unwrap();
    db.batch_execute("BEGIN; LOCK TABLE \"LiteLLM_VerificationToken\" IN ACCESS EXCLUSIVE MODE")
        .await
        .expect("the other connection holds the lock");
    let limited = harness.pool_with_size(1);
    let client = TableClient::new(limited.clone());
    let ports = Ports::new();
    let ops = ports.operations_with(
        &client,
        TableSettings {
            team_list_limit: DEFAULT_TEAM_LIST_LIMIT,
            acquire_timeout: DEFAULT_ACQUIRE_TIMEOUT,
            query_timeout: Duration::from_millis(50),
        },
    );
    let rows = vec![BTreeMap::from([
        (TokenColumn::Token, TokenValue::Text(String::from("tok-1"))),
        (
            TokenColumn::KeyAlias,
            TokenValue::Text(String::from("changed")),
        ),
    ])];
    let error = ops
        .update_many_keys(&rows)
        .await
        .expect_err("the locked batch cannot finish");
    // the first try always runs on an idle connection, so the statement, not the begin, is what times out
    assert!(matches!(
        error,
        TableOpsError::Timeout { deadline, .. } if deadline == Duration::from_millis(50)
    ));
    let events = ports.sink.events();
    assert!(
        events
            .iter()
            .any(|event| event.message.contains("the statement did not finish")),
        "the failed batch reports a statement timeout: {events:?}"
    );
    assert_eq!(
        limited.status().size,
        0,
        "a timed-out connection is discarded"
    );
    let fresh = limited.get().await.unwrap();
    wait_for_idle(&fresh, "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock'").await;
    drop(fresh);
    db.batch_execute("ROLLBACK")
        .await
        .expect("the lock is released");
    let alias: String = db
        .query_one(
            "SELECT key_alias FROM \"LiteLLM_VerificationToken\" WHERE token = 'tok-1'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        alias, "original",
        "a timed-out batch cannot write after the lock is released"
    );
    harness.close().await;
}

#[tokio::test]
async fn a_call_with_no_free_connection_times_out_waiting_for_one() {
    let harness = Harness::start().await;
    let limited = harness.pool_with_size(1);
    let held = limited.get().await.expect("the only connection");
    let client = TableClient::new(limited.clone());
    let ports = Ports::new();
    let ops = ports.operations_with(
        &client,
        TableSettings {
            team_list_limit: DEFAULT_TEAM_LIST_LIMIT,
            acquire_timeout: Duration::from_millis(20),
            query_timeout: DEFAULT_QUERY_TIMEOUT,
        },
    );
    let error = ops
        .user_unique("u-1")
        .await
        .expect_err("the pool has no free connection");
    assert!(matches!(
        error,
        TableOpsError::Timeout {
            operation: "acquire",
            deadline,
        } if deadline == Duration::from_millis(20)
    ));
    assert_eq!(ports.sink.events().len(), 3);
    drop(held);
    limited.close();
    harness.close().await;
}
