#![cfg(test)]
use crate::table_ops::deletes::TableDeletes as _;
use crate::table_ops::generic::GenericReads as _;
use crate::table_ops::inserts::TableInserts as _;
use crate::table_ops::reads::users::UserReads as _;
use crate::table_ops::updates::team::TeamUpdates as _;

use std::collections::BTreeMap;
use std::convert::Infallible;

use secrecy::SecretString;

use super::support::{Harness, Ports, TableClient, json};
use crate::key_repository::table::TokenValue;
use crate::table_ops::columns::TeamColumn;
use crate::table_ops::generic::ConfigFilter;
use crate::table_ops::reads::users::UserFilter;
use crate::table_ops::{OperationCallType, TableOpsError};

const NO_DB_MESSAGE: &str =
    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";

#[tokio::test]
async fn get_data_retries_three_times_and_reports_each_try() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.batch_execute("DROP TABLE \"LiteLLM_UserTable\" CASCADE")
        .await
        .expect("the user table drops");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let error = ops
        .user_unique("u-1")
        .await
        .expect_err("the user table is gone");
    assert!(matches!(error, TableOpsError::Database { .. }));
    let events = ports.sink.events();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].call_type, OperationCallType::GetData);
    assert_eq!(events[2].call_type, OperationCallType::GetData);
    assert_eq!(events[0].message, "the database operation failed");
    assert!(
        events[0]
            .traceback
            .starts_with("LiteLLM Prisma Client Exception: Error with `get_data`: ")
    );
    assert!(events[0].traceback.contains("\nCaused by: "));
    harness.close().await;
}

#[tokio::test]
async fn get_generic_data_reports_one_try() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.batch_execute("DROP TABLE \"LiteLLM_Config\"")
        .await
        .expect("the config table drops");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let error = ops
        .generic_row(ConfigFilter::ParamName("general_settings"))
        .await
        .expect_err("the config table is gone");
    assert!(matches!(error, TableOpsError::Database { .. }));
    let events = ports.sink.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].call_type, OperationCallType::GetGenericData);
    assert!(
        events[0]
            .traceback
            .starts_with("LiteLLM Prisma Client Exception get_generic_data: ")
    );
    harness.close().await;
}

#[tokio::test]
async fn insert_data_retries_three_times_and_keeps_nothing_invalidated() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.batch_execute("DROP TABLE \"LiteLLM_Config\"")
        .await
        .expect("the config table drops");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let error = ops
        .insert_config(&[(String::from("general_settings"), json("{}"))])
        .await
        .expect_err("the config table is gone");
    assert!(matches!(error, TableOpsError::Database { .. }));
    let events = ports.sink.events();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].call_type, OperationCallType::InsertData);
    assert!(
        events[0]
            .traceback
            .starts_with("LiteLLM Prisma Client Exception in insert_data: ")
    );
    assert!(ports.invalidator.params().is_empty());
    harness.close().await;
}

#[tokio::test]
async fn update_data_retries_three_times() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.batch_execute("DROP TABLE \"LiteLLM_TeamTable\" CASCADE")
        .await
        .expect("the team table drops");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let data = BTreeMap::from([(TeamColumn::TeamId, TokenValue::Text(String::from("team-1")))]);
    let error = ops
        .update_team(Some("team-1"), &data, None)
        .await
        .expect_err("the team table is gone");
    assert!(matches!(error, TableOpsError::Database { .. }));
    let events = ports.sink.events();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].call_type, OperationCallType::UpdateData);
    assert!(
        events[0]
            .traceback
            .starts_with("LiteLLM Prisma Client Exception - update_data: ")
    );
    harness.close().await;
}

#[tokio::test]
async fn delete_data_retries_three_times() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.batch_execute("DROP TABLE \"LiteLLM_VerificationToken\" CASCADE")
        .await
        .expect("the key table drops");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let error = ops
        .delete_keys(&[SecretString::from("sk-abc")], None)
        .await
        .expect_err("the key table is gone");
    assert!(matches!(error, TableOpsError::Database { .. }));
    let events = ports.sink.events();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].call_type, OperationCallType::DeleteData);
    assert!(
        events[0]
            .traceback
            .starts_with("LiteLLM Prisma Client Exception - delete_data: ")
    );
    harness.close().await;
}

#[tokio::test]
async fn an_offline_read_reports_no_db_connected_on_every_try() {
    let ports = Ports::new();
    let ops = ports.offline();
    let error = ops
        .user_unique("u-1")
        .await
        .expect_err("there is no database");
    assert!(matches!(error, TableOpsError::<Infallible>::NoDbConnected));
    let events = ports.sink.events();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].call_type, OperationCallType::GetData);
    assert_eq!(events[0].message, NO_DB_MESSAGE);
    assert_eq!(
        events[0].traceback,
        format!("LiteLLM Prisma Client Exception: Error with `get_data`: {NO_DB_MESSAGE}")
    );
}

#[tokio::test]
async fn an_offline_generic_read_reports_no_db_connected_once() {
    let ports = Ports::new();
    let ops = ports.offline();
    let error = ops
        .generic_row(UserFilter::UserId("u-1"))
        .await
        .expect_err("there is no database");
    assert!(matches!(error, TableOpsError::<Infallible>::NoDbConnected));
    let events = ports.sink.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].call_type, OperationCallType::GetGenericData);
    assert_eq!(events[0].message, NO_DB_MESSAGE);
}
