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

pub(super) const ROLLBACK_FIELDS: [[(TokenColumn, &str); 2]; 2] = [
    [
        (TokenColumn::Token, "sk-tok-1"),
        (TokenColumn::KeyAlias, "changed"),
    ],
    [
        (TokenColumn::Token, "sk-tok-2"),
        (TokenColumn::Spend, "not-a-number"),
    ],
];

pub(super) const TEAM_CLOCK_FIELDS: [[(TeamColumn, &str); 2]; 2] = [
    [
        (TeamColumn::TeamId, "team-1"),
        (TeamColumn::TeamAlias, "stamped"),
    ],
    [
        (TeamColumn::TeamId, "team-2"),
        (TeamColumn::TeamAlias, "explicit"),
    ],
];

pub(super) fn text_fields<Column, const N: usize>(
    fields: [(Column, &str); N],
) -> BTreeMap<Column, TokenValue>
where
    Column: Ord,
{
    fields
        .into_iter()
        .map(|(column, value)| (column, TokenValue::Text(value.to_owned())))
        .collect()
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
    let mut row = text_fields([
        (SpendColumn::RequestId, "req-1"),
        (SpendColumn::CallType, "completion"),
        (SpendColumn::ApiKey, "sk-fixture"),
        (SpendColumn::Model, "gpt-4"),
        (SpendColumn::User, "u-1"),
    ]);
    row.extend([
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
    ]);
    row
}

pub(super) const NOTIFICATION_FIELDS: [(NotificationColumn, &str); 4] = [
    (NotificationColumn::RequestId, "req-n"),
    (NotificationColumn::UserId, "u-1"),
    (NotificationColumn::Justification, "needs access"),
    (NotificationColumn::Status, "pending"),
];

pub(super) fn user_batch_rows() -> Vec<BTreeMap<UserColumn, TokenValue>> {
    let mut updated = text_fields([(UserColumn::UserId, "u-1")]);
    updated.extend([
        (UserColumn::Spend, TokenValue::Number(Decimal::new(25, 1))),
        (UserColumn::UserAlias, TokenValue::Null),
    ]);
    vec![
        updated,
        text_fields([(UserColumn::UserId, "u-2"), (UserColumn::UserAlias, "new")]),
    ]
}

pub(super) fn budget_payload(id: &str) -> BTreeMap<BudgetColumn, TokenValue> {
    text_fields([
        (BudgetColumn::BudgetId, id),
        (BudgetColumn::CreatedBy, "created"),
        (BudgetColumn::UpdatedBy, "updated"),
    ])
}
