#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use super::support::{Harness, Ports, TableClient, at, json};
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_user_writes::columns::UserColumn;
use crate::table_ops::columns::ConfigColumn;
use crate::table_ops::columns::spend_notification::SpendColumn;
use crate::table_ops::generic::GenericReads as _;
use crate::table_ops::generic::columns::ColumnFilter;
use crate::table_ops::{OperationCallType, TableOps, TableOpsError};

#[rstest]
#[case::decimal(TokenColumn::MaxBudget, TokenValue::Number(Decimal::new(105, 1)))]
#[case::null(TokenColumn::KeyAlias, TokenValue::Null)]
#[case::integer(TokenColumn::MaxParallelRequests, TokenValue::Integer(7))]
#[case::timestamp(TokenColumn::Expires, TokenValue::Timestamp(at(1_700_000_000)))]
#[case::array(TokenColumn::Models, TokenValue::TextArray(vec![String::from("gpt")]))]
#[case::json(TokenColumn::Metadata, TokenValue::Json(json(r#"{"a":1}"#)))]
#[case::boolean(TokenColumn::Blocked, TokenValue::Bool(true))]
#[case::text(TokenColumn::UserId, TokenValue::Text(String::from("u-1")))]
#[tokio::test]
async fn generic_key_columns_use_typed_equality(
    #[case] column: TokenColumn,
    #[case] value: TokenValue,
) {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.unwrap();
    db.batch_execute(r#"INSERT INTO "LiteLLM_VerificationToken"
        (token, max_budget, key_alias, max_parallel_requests, expires, models, metadata, blocked, user_id)
        VALUES ('other', 11.0, 'alias', 8, '2020-01-01', ARRAY['other'], '{"a":2}', false, 'u-2'),
        ('matching', 10.5, NULL, 7, '2023-11-14T22:13:20', ARRAY['gpt'], '{"a":1}', true, 'u-1')"#).await.unwrap();
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let row = ports
        .operations(&client)
        .generic_row(ColumnFilter {
            column,
            value: &value,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.token.as_ref().unwrap().expose_secret(), "matching");
    harness.close().await;
}

#[tokio::test]
async fn generic_columns_cover_users_config_and_spend() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.unwrap();
    db.batch_execute(r#"INSERT INTO "LiteLLM_UserTable" (user_id, max_budget) VALUES ('user', 10.5);
        INSERT INTO "LiteLLM_Config" (param_name, param_value) VALUES ('null', NULL), ('json', '{"a":1}');
        INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, "startTime", "endTime", total_tokens)
        VALUES ('request', 'acompletion', '2024-01-01', '2024-01-01', 17)"#).await.unwrap();
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    assert_user_and_config_filters(&ops).await;
    let spend = ops
        .generic_row(ColumnFilter {
            column: SpendColumn::TotalTokens,
            value: &TokenValue::Integer(17),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(spend.request_id, "request");
    assert!(
        ops.generic_row(ColumnFilter {
            column: SpendColumn::TotalTokens,
            value: &TokenValue::Integer(18),
        })
        .await
        .unwrap()
        .is_none()
    );
    harness.close().await;
}

#[tokio::test]
async fn generic_column_type_errors_report_once() {
    let ports = Ports::new();
    let error = ports
        .offline()
        .generic_row(ColumnFilter {
            column: TokenColumn::Blocked,
            value: &TokenValue::Integer(1),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        TableOpsError::WrongType { column: "blocked" }
    ));
    let events = ports.sink.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].call_type, OperationCallType::GetGenericData);
}

async fn assert_user_and_config_filters(ops: &TableOps<'_, TableClient>) {
    let user = ops
        .generic_row(ColumnFilter {
            column: UserColumn::MaxBudget,
            value: &TokenValue::Number(Decimal::new(105, 1)),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.user_id, "user");
    let config = ops
        .generic_row(ColumnFilter {
            column: ConfigColumn::ParamValue,
            value: &TokenValue::Null,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(config.param_name, "null");
}
