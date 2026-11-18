#![cfg(test)]

use crate::budget_permission_rows::JsonData;
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_user_writes::columns::UserColumn;
use crate::table_ops::columns::TeamColumn;
use crate::table_ops::columns::budget_enduser::BudgetColumn;
use crate::table_ops::columns::spend_notification::{NotificationColumn, SpendColumn};
use crate::table_ops::tests::support::{at, json};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

pub(super) fn team_clock_rows() -> Vec<BTreeMap<TeamColumn, TokenValue>> {
    vec![
        BTreeMap::from([
            (TeamColumn::TeamId, TokenValue::Text(String::from("team-1"))),
            (
                TeamColumn::TeamAlias,
                TokenValue::Text(String::from("stamped")),
            ),
        ]),
        BTreeMap::from([
            (TeamColumn::TeamId, TokenValue::Text(String::from("team-2"))),
            (
                TeamColumn::TeamAlias,
                TokenValue::Text(String::from("explicit")),
            ),
            (
                TeamColumn::UpdatedAt,
                TokenValue::Timestamp(at(1_700_000_000)),
            ),
        ]),
    ]
}

pub(super) fn config_values() -> [(String, JsonData); 2] {
    [
        (
            String::from("general_settings"),
            json(r#"{"store_model_in_db": true}"#),
        ),
        (
            String::from("litellm_settings"),
            json(r#"{"num_retries": 3}"#),
        ),
    ]
}

pub(super) fn spend_payload() -> BTreeMap<SpendColumn, TokenValue> {
    BTreeMap::from([
        (
            SpendColumn::RequestId,
            TokenValue::Text(String::from("req-1")),
        ),
        (
            SpendColumn::CallType,
            TokenValue::Text(String::from("completion")),
        ),
        (
            SpendColumn::ApiKey,
            TokenValue::Text(String::from("sk-fixture")),
        ),
        (SpendColumn::Spend, TokenValue::Number(Decimal::new(15, 1))),
        (SpendColumn::TotalTokens, TokenValue::Integer(10)),
        (
            SpendColumn::StartTime,
            TokenValue::Timestamp(at(1_000_000_000)),
        ),
        (
            SpendColumn::EndTime,
            TokenValue::Timestamp(at(1_000_000_000)),
        ),
        (SpendColumn::Model, TokenValue::Text(String::from("gpt-4"))),
        (SpendColumn::User, TokenValue::Text(String::from("u-1"))),
    ])
}

pub(super) fn notification_payload() -> BTreeMap<NotificationColumn, TokenValue> {
    BTreeMap::from([
        (
            NotificationColumn::RequestId,
            TokenValue::Text(String::from("req-n")),
        ),
        (
            NotificationColumn::UserId,
            TokenValue::Text(String::from("u-1")),
        ),
        (
            NotificationColumn::Models,
            TokenValue::TextArray(vec![String::from("gpt-4")]),
        ),
        (
            NotificationColumn::Justification,
            TokenValue::Text(String::from("needs access")),
        ),
        (
            NotificationColumn::Status,
            TokenValue::Text(String::from("pending")),
        ),
    ])
}

pub(super) fn rollback_rows() -> Vec<BTreeMap<TokenColumn, TokenValue>> {
    vec![
        BTreeMap::from([
            (
                TokenColumn::Token,
                TokenValue::Text(String::from("sk-tok-1")),
            ),
            (
                TokenColumn::KeyAlias,
                TokenValue::Text(String::from("changed")),
            ),
        ]),
        BTreeMap::from([
            (
                TokenColumn::Token,
                TokenValue::Text(String::from("sk-tok-2")),
            ),
            (
                TokenColumn::Spend,
                TokenValue::Text(String::from("not-a-number")),
            ),
        ]),
    ]
}

pub(super) fn user_batch_rows() -> Vec<BTreeMap<UserColumn, TokenValue>> {
    vec![
        BTreeMap::from([
            (UserColumn::UserId, TokenValue::Text(String::from("u-1"))),
            (UserColumn::Spend, TokenValue::Number(Decimal::new(25, 1))),
            (UserColumn::UserAlias, TokenValue::Null),
        ]),
        BTreeMap::from([
            (UserColumn::UserId, TokenValue::Text(String::from("u-2"))),
            (UserColumn::UserAlias, TokenValue::Text(String::from("new"))),
        ]),
    ]
}

pub(super) fn budget_batch_rows() -> Vec<BTreeMap<BudgetColumn, TokenValue>> {
    let mut updated = budget_payload("b-1");
    updated.insert(
        BudgetColumn::MaxBudget,
        TokenValue::Number(Decimal::new(1000, 2)),
    );
    vec![updated, budget_payload("b-2")]
}

pub(super) fn budget_payload(id: &str) -> BTreeMap<BudgetColumn, TokenValue> {
    BTreeMap::from([
        (BudgetColumn::BudgetId, TokenValue::Text(id.to_owned())),
        (
            BudgetColumn::CreatedBy,
            TokenValue::Text(String::from("created")),
        ),
        (
            BudgetColumn::UpdatedBy,
            TokenValue::Text(String::from("updated")),
        ),
    ])
}
