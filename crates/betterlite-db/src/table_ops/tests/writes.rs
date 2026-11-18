#![cfg(test)]
use crate::table_ops::deletes::TableDeletes as _;
use crate::table_ops::inserts::TableInserts as _;
use crate::table_ops::reads::resets::ResetReads as _;
use crate::table_ops::reads::users::UserReads as _;
use crate::table_ops::updates::batches::BatchUpdates as _;
use crate::table_ops::updates::team::TeamUpdates as _;

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio_postgres::Client;

use super::support::{Harness, ManualClock, Ports, TableClient, at, json, seed};
use crate::budget_permission_rows::JsonData;
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_user_writes::columns::UserColumn;
use crate::key_user_writes::hash::hash_token;
use crate::table_ops::columns::TeamColumn;
use crate::table_ops::columns::budget_enduser::BudgetColumn;
use crate::table_ops::columns::budget_enduser::EndUserColumn;
use crate::table_ops::columns::spend_notification::NotificationColumn;
use crate::table_ops::columns::spend_notification::SpendColumn;
use crate::table_ops::{OperationCallType, TableOpsError};

const KEY_INSERT: &str = "INSERT INTO \"LiteLLM_VerificationToken\" (\"token\", \"key_alias\", \"user_id\", \"spend\", \"models\") VALUES ($1, $2, $3, $4, ARRAY[]::text[])";
const TEAM_INSERT: &str = "INSERT INTO \"LiteLLM_TeamTable\" (\"team_id\", \"admins\", \"members\", \"models\") VALUES ($1, ARRAY[]::text[], ARRAY[]::text[], ARRAY[]::text[])";
const USER_INSERT: &str = "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\", \"user_alias\", \"updated_at\") VALUES ($1, $2, '2000-01-01'::timestamp)";
const BUDGET_INSERT: &str = "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\", \"created_by\", \"updated_by\", \"updated_at\") VALUES ($1, 'created', 'updated', '2000-01-01'::timestamp)";
const END_USER_INSERT: &str = "INSERT INTO \"LiteLLM_EndUserTable\" (\"user_id\") VALUES ($1)";

fn team_payload(team_id: &str, alias: &str) -> BTreeMap<TeamColumn, TokenValue> {
    BTreeMap::from([
        (TeamColumn::TeamId, TokenValue::Text(team_id.to_owned())),
        (TeamColumn::TeamAlias, TokenValue::Text(alias.to_owned())),
        (TeamColumn::Admins, TokenValue::TextArray(Vec::new())),
        (TeamColumn::Members, TokenValue::TextArray(Vec::new())),
        (TeamColumn::Models, TokenValue::TextArray(Vec::new())),
    ])
}

async fn text_column(db: &Client, sql: &str, parameter: &str) -> Option<String> {
    db.query_opt(sql, &[&parameter])
        .await
        .expect("the column query runs")
        .map(|row| row.get(0))
}

#[tokio::test]
async fn insert_team_dumps_the_map_fields_to_json_text() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let mut data = team_payload("team-2", "maps");
    data.insert(
        TeamColumn::Metadata,
        TokenValue::Json(json(r#"{"department":"eng"}"#)),
    );
    data.insert(TeamColumn::MembersWithRoles, TokenValue::Json(json("[]")));
    data.insert(TeamColumn::BudgetLimits, TokenValue::Json(json("[]")));
    let created = ops.insert_team(&data).await.expect("the team insert runs");
    let metadata = created
        .fields
        .base
        .metadata
        .expect("the stored metadata decodes");
    assert_eq!(
        metadata.get("department"),
        Some(&JsonData::String(String::from("eng")))
    );
    assert!(created.fields.base.members_with_roles.is_empty());
    assert_eq!(created.fields.base.budget_limits, Some(Vec::new()));
    let row = db
        .query_one(
            "SELECT jsonb_typeof(metadata), jsonb_typeof(members_with_roles), jsonb_typeof(budget_limits) FROM \"LiteLLM_TeamTable\" WHERE team_id = 'team-2'",
            &[],
        )
        .await
        .expect("the type query runs");
    assert_eq!(row.get::<_, Option<String>>(0).as_deref(), Some("string"));
    assert_eq!(row.get::<_, Option<String>>(1).as_deref(), Some("string"));
    assert_eq!(row.get::<_, Option<String>>(2).as_deref(), Some("string"));
    harness.close().await;
}

#[tokio::test]
async fn update_team_sets_the_values_and_keeps_budget_limits_raw() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let mut data = team_payload("team-3", "created");
    data.insert(
        TeamColumn::Metadata,
        TokenValue::Json(json(r#"{"department":"eng"}"#)),
    );
    data.insert(TeamColumn::MembersWithRoles, TokenValue::Json(json("[]")));
    data.insert(TeamColumn::BudgetLimits, TokenValue::Json(json("[]")));
    ops.insert_team(&data).await.expect("the team insert runs");
    let mut changed = data;
    changed.insert(
        TeamColumn::TeamAlias,
        TokenValue::Text(String::from("updated")),
    );
    let result = ops
        .update_team(Some("team-3"), &changed, None)
        .await
        .expect("the team update runs");
    assert_eq!(result.team_id, "team-3");
    assert_eq!(
        result.data.fields.base.team_alias.as_deref(),
        Some("updated")
    );
    let row = db
        .query_one(
            "SELECT jsonb_typeof(metadata), jsonb_typeof(budget_limits) FROM \"LiteLLM_TeamTable\" WHERE team_id = 'team-3'",
            &[],
        )
        .await
        .expect("the type query runs");
    assert_eq!(row.get::<_, Option<String>>(0).as_deref(), Some("string"));
    assert_eq!(row.get::<_, Option<String>>(1).as_deref(), Some("array"));
    harness.close().await;
}

#[tokio::test]
async fn update_team_inserts_and_falls_back_to_the_rows_team_id() {
    let harness = Harness::start().await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let created = ops
        .update_team(None, &team_payload("team-2", "fallback"), None)
        .await
        .expect("the team upsert runs");
    assert_eq!(created.team_id, "team-2");
    assert_eq!(
        created.data.fields.base.team_alias.as_deref(),
        Some("fallback")
    );
    let patched = ops
        .update_team(
            Some("team-2"),
            &team_payload("team-2", "created"),
            Some(&BTreeMap::from([(
                TeamColumn::TeamAlias,
                TokenValue::Text(String::from("patched")),
            )])),
        )
        .await
        .expect("the team update runs");
    assert_eq!(
        patched.data.fields.base.team_alias.as_deref(),
        Some("patched")
    );
    let mut unnamed = team_payload("ignored", "named");
    unnamed.remove(&TeamColumn::TeamId);
    let named = ops
        .update_team(Some("team-9"), &unnamed, None)
        .await
        .expect("the named team insert runs");
    assert_eq!(named.team_id, "team-9");
    assert_eq!(named.data.fields.base.team_id, "team-9");
    assert_eq!(named.data.fields.base.team_alias.as_deref(), Some("named"));
    harness.close().await;
}

#[tokio::test]
async fn update_team_needs_a_typed_team_id() {
    let harness = Harness::start().await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let missing = ops
        .update_team(None, &BTreeMap::new(), None)
        .await
        .expect_err("a team without an id cannot be written");
    assert!(matches!(
        missing,
        TableOpsError::MissingField { field: "team_id" }
    ));
    let failed = ports.sink.events();
    assert_eq!(failed.len(), 3);
    assert_eq!(failed[0].call_type, OperationCallType::UpdateData);
    assert_eq!(
        failed[0].message,
        "the write payload needs the team_id value"
    );
    let wrong = ops
        .update_team(
            None,
            &BTreeMap::from([(TeamColumn::TeamId, TokenValue::Integer(1))]),
            None,
        )
        .await
        .expect_err("a numeric team id is not writable");
    assert!(matches!(
        wrong,
        TableOpsError::WrongType { column: "team_id" }
    ));
    let rejected = ports.sink.events();
    assert_eq!(rejected.len(), 3);
    assert_eq!(
        rejected[0].message,
        "the write value for team_id does not fit its column type"
    );
    harness.close().await;
}

#[tokio::test]
async fn update_team_targets_the_resolved_id_not_the_payload_id() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, TEAM_INSERT, &[&"team-a"]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let moved = ops
        .update_team(Some("team-a"), &team_payload("team-b", "moved"), None)
        .await
        .expect("the existing team updates under the resolved id");
    assert_eq!(moved.team_id, "team-a");
    assert_eq!(moved.data.fields.base.team_id, "team-b");
    let renamed = text_column(
        &db,
        "SELECT team_alias FROM \"LiteLLM_TeamTable\" WHERE team_id = $1",
        "team-b",
    )
    .await;
    assert_eq!(renamed.as_deref(), Some("moved"));
    let inserted = ops
        .update_team(Some("team-c"), &team_payload("team-d", "fresh"), None)
        .await
        .expect("the missing team inserts the payload row");
    assert_eq!(inserted.team_id, "team-c");
    assert_eq!(inserted.data.fields.base.team_id, "team-d");
    let fresh = text_column(
        &db,
        "SELECT team_alias FROM \"LiteLLM_TeamTable\" WHERE team_id = $1",
        "team-d",
    )
    .await;
    assert_eq!(fresh.as_deref(), Some("fresh"));
    let absent = text_column(
        &db,
        "SELECT team_alias FROM \"LiteLLM_TeamTable\" WHERE team_id = $1",
        "team-c",
    )
    .await;
    assert_eq!(absent, None);
    harness.close().await;
}

#[tokio::test]
async fn update_team_stamps_updated_at_from_the_injected_clock() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, TEAM_INSERT, &[&"team-at"]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::with_clock(Box::new(ManualClock::new(1_800_000_000)));
    let ops = ports.operations(&client);
    let stamped = ops
        .update_team(Some("team-at"), &team_payload("team-at", "stamped"), None)
        .await
        .expect("the team update runs");
    assert_eq!(stamped.data.fields.updated_at, Some(at(1_800_000_000)));
    let mut explicit = team_payload("team-at", "explicit");
    explicit.insert(
        TeamColumn::UpdatedAt,
        TokenValue::Timestamp(at(1_700_000_000)),
    );
    let kept = ops
        .update_team(Some("team-at"), &explicit, None)
        .await
        .expect("the explicit update runs");
    assert_eq!(kept.data.fields.updated_at, Some(at(1_700_000_000)));
    harness.close().await;
}

#[tokio::test]
async fn an_empty_team_update_leaves_updated_at_alone() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_TeamTable\" (\"team_id\", \"admins\", \"members\", \"models\", \"updated_at\") VALUES ('team-empty', ARRAY[]::text[], ARRAY[]::text[], ARRAY[]::text[], '2000-01-01'::timestamp)",
        &[],
    )
    .await
    .expect("the team row commits");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::with_clock(Box::new(ManualClock::new(1_800_000_000)));
    let ops = ports.operations(&client);
    let unchanged = ops
        .update_team(Some("team-empty"), &BTreeMap::new(), Some(&BTreeMap::new()))
        .await
        .expect("the empty update runs");
    assert_eq!(unchanged.team_id, "team-empty");
    let stamp = text_column(
        &db,
        "SELECT to_char(updated_at, 'YYYY-MM-DD HH24:MI:SS') FROM \"LiteLLM_TeamTable\" WHERE team_id = $1",
        "team-empty",
    )
    .await;
    assert_eq!(stamp.as_deref(), Some("2000-01-01 00:00:00"));
    harness.close().await;
}

#[tokio::test]
async fn update_many_teams_stamps_updated_at_from_the_injected_clock() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, TEAM_INSERT, &[&"team-1"]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::with_clock(Box::new(ManualClock::new(1_800_000_000)));
    let ops = ports.operations(&client);
    let rows = vec![
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
    ];
    ops.update_many_teams(&rows)
        .await
        .expect("the team batch runs");
    let stamped = text_column(
        &db,
        "SELECT to_char(updated_at, 'YYYY-MM-DD HH24:MI:SS') FROM \"LiteLLM_TeamTable\" WHERE team_id = $1",
        "team-1",
    )
    .await;
    assert_eq!(stamped.as_deref(), Some("2027-01-15 08:00:00"));
    let explicit = text_column(
        &db,
        "SELECT to_char(updated_at, 'YYYY-MM-DD HH24:MI:SS') FROM \"LiteLLM_TeamTable\" WHERE team_id = $1",
        "team-2",
    )
    .await;
    assert_eq!(explicit.as_deref(), Some("2023-11-14 22:13:20"));
    harness.close().await;
}

#[tokio::test]
async fn update_team_writes_the_integer_columns() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    let model_id: i32 = db
        .query_one(
            "INSERT INTO \"LiteLLM_ModelTable\" (\"aliases\", \"created_by\", \"updated_by\") VALUES ('{}'::jsonb, 'c', 'u') RETURNING \"id\"",
            &[],
        )
        .await
        .expect("the model row commits")
        .get(0);
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let mut data = team_payload("team-int", "ints");
    data.insert(TeamColumn::MaxParallelRequests, TokenValue::Integer(150));
    data.insert(
        TeamColumn::ModelId,
        TokenValue::Integer(i64::from(model_id)),
    );
    let created = ops.insert_team(&data).await.expect("the team insert runs");
    assert_eq!(created.fields.max_parallel_requests, Some(150));
    assert_eq!(created.fields.model_id, Some(i64::from(model_id)));
    let mut changed = data;
    changed.insert(TeamColumn::MaxParallelRequests, TokenValue::Integer(160));
    let updated = ops
        .update_team(Some("team-int"), &changed, None)
        .await
        .expect("the team update runs");
    assert_eq!(updated.data.fields.max_parallel_requests, Some(160));
    harness.close().await;
}

#[tokio::test]
async fn update_many_budgets_reads_back_the_json_text_budget_map() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\", \"created_by\", \"updated_by\", \"budget_duration\", \"budget_reset_at\") VALUES ('b-3', 'c', 'u', '1d', '2020-01-01'::timestamp)",
        &[],
    )
    .await
    .expect("the budget row commits");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let rows = vec![BTreeMap::from([
        (
            BudgetColumn::BudgetId,
            TokenValue::Text(String::from("b-3")),
        ),
        (
            BudgetColumn::ModelMaxBudget,
            TokenValue::Json(json(r#"{"gpt-4":{"max_budget":10}}"#)),
        ),
        (BudgetColumn::MaxParallelRequests, TokenValue::Integer(7)),
    ])];
    ops.update_many_budgets(&rows)
        .await
        .expect("the budget batch runs");
    let found = ops
        .budgets_due_for_reset(at(2_000_000_000).utc(), None)
        .await
        .expect("the budget reset read runs");
    let budget = found
        .into_iter()
        .find(|row| row.budget.budget_id.as_deref() == Some("b-3"))
        .expect("the budget row reads back");
    assert_eq!(budget.budget.max_parallel_requests, Some(7));
    let limits = budget
        .budget
        .model_max_budget
        .expect("the budget map decodes");
    assert_eq!(limits.get("gpt-4"), Some(&json(r#"{"max_budget":10}"#)));
    harness.close().await;
}

#[tokio::test]
async fn insert_config_writes_json_text_and_invalidates_each_param() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let data = [
        (
            String::from("general_settings"),
            json(r#"{"store_model_in_db": true}"#),
        ),
        (
            String::from("litellm_settings"),
            json(r#"{"num_retries": 3}"#),
        ),
    ];
    ops.insert_config(&data)
        .await
        .expect("the config insert runs");
    assert_eq!(
        ports.invalidator.params(),
        vec![
            String::from("general_settings"),
            String::from("litellm_settings")
        ]
    );
    let row = db
        .query_one(
            "SELECT jsonb_typeof(param_value), param_value #>> '{}' FROM \"LiteLLM_Config\" WHERE param_name = 'general_settings'",
            &[],
        )
        .await
        .expect("the config row reads");
    assert_eq!(row.get::<_, Option<String>>(0).as_deref(), Some("string"));
    assert_eq!(
        row.get::<_, Option<String>>(1).as_deref(),
        Some(r#"{"store_model_in_db":true}"#)
    );
    ops.insert_config(&[(
        String::from("general_settings"),
        json(r#"{"store_model_in_db": false}"#),
    )])
    .await
    .expect("the config update runs");
    assert_eq!(
        ports.invalidator.params(),
        vec![String::from("general_settings")]
    );
    let stored = text_column(
        &db,
        "SELECT param_value #>> '{}' FROM \"LiteLLM_Config\" WHERE param_name = $1",
        "general_settings",
    )
    .await;
    assert_eq!(stored.as_deref(), Some(r#"{"store_model_in_db":false}"#));
    harness.close().await;
}

#[tokio::test]
async fn insert_spend_writes_the_row_and_keeps_an_existing_one() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let mut data = BTreeMap::from([
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
    ]);
    data.insert(
        SpendColumn::Metadata,
        TokenValue::Json(json(r#"{"trace":"abc"}"#)),
    );
    let row = ops
        .insert_spend(&data)
        .await
        .expect("the spend insert runs");
    assert_eq!(row.request_id, "req-1");
    assert_eq!(row.call_type, "completion");
    assert_eq!(row.spend, Decimal::new(15, 1));
    assert_eq!(row.total_tokens, 10);
    assert!(matches!(row.metadata, Some(JsonData::String(_))));
    let kind = text_column(
        &db,
        "SELECT jsonb_typeof(metadata) FROM \"LiteLLM_SpendLogs\" WHERE request_id = $1",
        "req-1",
    )
    .await;
    assert_eq!(kind.as_deref(), Some("string"));
    let mut replaced = data;
    replaced.insert(SpendColumn::Model, TokenValue::Text(String::from("gpt-5")));
    let kept = ops
        .insert_spend(&replaced)
        .await
        .expect("the conflicting insert runs");
    assert_eq!(kept.model, "gpt-4");
    harness.close().await;
}

#[tokio::test]
async fn insert_user_notification_writes_the_row_and_keeps_the_first_status() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let data = BTreeMap::from([
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
    ]);
    let row = ops
        .insert_user_notification(&data)
        .await
        .expect("the notification insert runs");
    assert_eq!(row.models, Some(vec![String::from("gpt-4")]));
    assert_eq!(row.status, "pending");
    let mut approved = data;
    approved.insert(
        NotificationColumn::Status,
        TokenValue::Text(String::from("approved")),
    );
    let kept = ops
        .insert_user_notification(&approved)
        .await
        .expect("the conflicting insert runs");
    assert_eq!(kept.status, "pending");
    let status = text_column(
        &db,
        "SELECT status FROM \"LiteLLM_UserNotifications\" WHERE request_id = $1",
        "req-n",
    )
    .await;
    assert_eq!(status.as_deref(), Some("pending"));
    harness.close().await;
}

#[tokio::test]
async fn update_many_keys_hashes_the_token_column() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, KEY_INSERT, &[&hash_token("sk-abc"), &"old", &"", &0.0]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let rows = vec![BTreeMap::from([
        (TokenColumn::Token, TokenValue::Text(String::from("sk-abc"))),
        (TokenColumn::KeyAlias, TokenValue::Text(String::from("new"))),
        (
            TokenColumn::Metadata,
            TokenValue::Json(json(r#"{"team":"eng"}"#)),
        ),
    ])];
    ops.update_many_keys(&rows)
        .await
        .expect("the key batch runs");
    let row = db
        .query_one(
            "SELECT key_alias, jsonb_typeof(metadata) FROM \"LiteLLM_VerificationToken\" WHERE token = $1",
            &[&hash_token("sk-abc")],
        )
        .await
        .expect("the key row reads");
    assert_eq!(row.get::<_, Option<String>>(0).as_deref(), Some("new"));
    assert_eq!(row.get::<_, Option<String>>(1).as_deref(), Some("string"));
    let stamped = db
        .query_one(
            "SELECT updated_at IS NOT NULL FROM \"LiteLLM_VerificationToken\" WHERE token = $1",
            &[&hash_token("sk-abc")],
        )
        .await
        .expect("the stamp query runs")
        .get::<_, Option<bool>>(0);
    assert_eq!(stamped, Some(true));
    harness.close().await;
}

#[tokio::test]
async fn update_many_keys_rolls_back_the_whole_batch() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(
        &db,
        KEY_INSERT,
        &[&hash_token("sk-tok-1"), &"first", &"", &0.0],
    )
    .await;
    seed(
        &db,
        KEY_INSERT,
        &[&hash_token("sk-tok-2"), &"second", &"", &0.0],
    )
    .await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let rows = vec![
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
    ];
    let error = ops
        .update_many_keys(&rows)
        .await
        .expect_err("the batch has a value the database rejects");
    assert!(matches!(error, TableOpsError::Database { .. }));
    let alias = db
        .query_one(
            "SELECT key_alias FROM \"LiteLLM_VerificationToken\" WHERE token = $1",
            &[&hash_token("sk-tok-1")],
        )
        .await
        .expect("the key row reads")
        .get::<_, Option<String>>(0);
    assert_eq!(alias.as_deref(), Some("first"));
    assert_eq!(ports.sink.events().len(), 3);
    harness.close().await;
}

#[tokio::test]
async fn update_many_users_sets_values_omits_nulls_and_inserts_rows() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, USER_INSERT, &[&"u-1", &"old"]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let rows = vec![
        BTreeMap::from([
            (UserColumn::UserId, TokenValue::Text(String::from("u-1"))),
            (UserColumn::Spend, TokenValue::Number(Decimal::new(25, 1))),
            (UserColumn::UserAlias, TokenValue::Null),
        ]),
        BTreeMap::from([
            (UserColumn::UserId, TokenValue::Text(String::from("u-2"))),
            (UserColumn::UserAlias, TokenValue::Text(String::from("new"))),
        ]),
    ];
    ops.update_many_users(&rows)
        .await
        .expect("the user batch runs");
    let updated = ops
        .user_unique("u-1")
        .await
        .expect("the user read runs")
        .expect("the user row");
    assert_eq!(updated.spend, Decimal::new(25, 1));
    assert_eq!(updated.user_alias.as_deref(), Some("old"));
    let inserted = ops
        .user_unique("u-2")
        .await
        .expect("the user read runs")
        .expect("the user row");
    assert_eq!(inserted.user_alias.as_deref(), Some("new"));
    let updated_stamp = text_column(
        &db,
        "SELECT to_char(updated_at, 'YYYY-MM-DD HH24:MI:SS') FROM \"LiteLLM_UserTable\" WHERE user_id = $1",
        "u-1",
    )
    .await;
    assert_ne!(updated_stamp.as_deref(), Some("2000-01-01 00:00:00"));
    let inserted_stamp = text_column(
        &db,
        "SELECT to_char(updated_at, 'YYYY-MM-DD HH24:MI:SS') FROM \"LiteLLM_UserTable\" WHERE user_id = $1",
        "u-2",
    )
    .await;
    assert!(inserted_stamp.is_some(), "the new user row is stamped");
    harness.close().await;
}

#[tokio::test]
async fn update_many_teams_sets_values_and_inserts_rows() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, TEAM_INSERT, &[&"team-1"]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let rows = vec![
        BTreeMap::from([
            (TeamColumn::TeamId, TokenValue::Text(String::from("team-1"))),
            (
                TeamColumn::TeamAlias,
                TokenValue::Text(String::from("renamed")),
            ),
        ]),
        BTreeMap::from([
            (TeamColumn::TeamId, TokenValue::Text(String::from("team-2"))),
            (
                TeamColumn::TeamAlias,
                TokenValue::Text(String::from("fresh")),
            ),
        ]),
    ];
    ops.update_many_teams(&rows)
        .await
        .expect("the team batch runs");
    let renamed = text_column(
        &db,
        "SELECT team_alias FROM \"LiteLLM_TeamTable\" WHERE team_id = $1",
        "team-1",
    )
    .await;
    assert_eq!(renamed.as_deref(), Some("renamed"));
    let fresh = text_column(
        &db,
        "SELECT team_alias FROM \"LiteLLM_TeamTable\" WHERE team_id = $1",
        "team-2",
    )
    .await;
    assert_eq!(fresh.as_deref(), Some("fresh"));
    harness.close().await;
}

#[tokio::test]
async fn update_many_budgets_sets_values_and_inserts_rows() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, BUDGET_INSERT, &[&"b-1"]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let rows = vec![
        BTreeMap::from([
            (
                BudgetColumn::BudgetId,
                TokenValue::Text(String::from("b-1")),
            ),
            (
                BudgetColumn::MaxBudget,
                TokenValue::Number(Decimal::new(1000, 2)),
            ),
        ]),
        BTreeMap::from([
            (
                BudgetColumn::BudgetId,
                TokenValue::Text(String::from("b-2")),
            ),
            (
                BudgetColumn::CreatedBy,
                TokenValue::Text(String::from("created")),
            ),
            (
                BudgetColumn::UpdatedBy,
                TokenValue::Text(String::from("updated")),
            ),
        ]),
    ];
    ops.update_many_budgets(&rows)
        .await
        .expect("the budget batch runs");
    let stored = text_column(
        &db,
        "SELECT max_budget::text FROM \"LiteLLM_BudgetTable\" WHERE budget_id = $1",
        "b-1",
    )
    .await;
    assert_eq!(stored.as_deref(), Some("10"));
    let inserted = text_column(
        &db,
        "SELECT updated_by FROM \"LiteLLM_BudgetTable\" WHERE budget_id = $1",
        "b-2",
    )
    .await;
    assert_eq!(inserted.as_deref(), Some("updated"));
    let updated_stamp = text_column(
        &db,
        "SELECT to_char(updated_at, 'YYYY-MM-DD HH24:MI:SS') FROM \"LiteLLM_BudgetTable\" WHERE budget_id = $1",
        "b-1",
    )
    .await;
    assert_ne!(updated_stamp.as_deref(), Some("2000-01-01 00:00:00"));
    let inserted_stamp = text_column(
        &db,
        "SELECT to_char(updated_at, 'YYYY-MM-DD HH24:MI:SS') FROM \"LiteLLM_BudgetTable\" WHERE budget_id = $1",
        "b-2",
    )
    .await;
    assert!(inserted_stamp.is_some(), "the new budget row is stamped");
    harness.close().await;
}

#[tokio::test]
async fn update_many_end_users_sets_values_and_inserts_rows() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, END_USER_INSERT, &[&"e-1"]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let rows = vec![
        BTreeMap::from([
            (EndUserColumn::UserId, TokenValue::Text(String::from("e-1"))),
            (
                EndUserColumn::Spend,
                TokenValue::Number(Decimal::new(35, 1)),
            ),
        ]),
        BTreeMap::from([
            (EndUserColumn::UserId, TokenValue::Text(String::from("e-2"))),
            (EndUserColumn::Alias, TokenValue::Text(String::from("new"))),
        ]),
    ];
    ops.update_many_end_users(&rows)
        .await
        .expect("the end user batch runs");
    let stored = text_column(
        &db,
        "SELECT spend::text FROM \"LiteLLM_EndUserTable\" WHERE user_id = $1",
        "e-1",
    )
    .await;
    assert_eq!(stored.as_deref(), Some("3.5"));
    let inserted = text_column(
        &db,
        "SELECT alias FROM \"LiteLLM_EndUserTable\" WHERE user_id = $1",
        "e-2",
    )
    .await;
    assert_eq!(inserted.as_deref(), Some("new"));
    harness.close().await;
}

#[tokio::test]
async fn delete_keys_hashes_tokens_and_applies_the_user_filter() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, KEY_INSERT, &[&hash_token("sk-a"), &"a", &"u-1", &0.0]).await;
    seed(&db, KEY_INSERT, &[&hash_token("sk-b"), &"b", &"u-2", &0.0]).await;
    seed(&db, KEY_INSERT, &[&"literal-key", &"c", &"u-2", &0.0]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let filtered = ops
        .delete_keys(
            &[SecretString::from("sk-a"), SecretString::from("sk-b")],
            Some("u-2"),
        )
        .await
        .expect("the filtered delete runs");
    assert_eq!(filtered, 1);
    let hashed = ops
        .delete_keys(&[SecretString::from("sk-a")], None)
        .await
        .expect("the hashed delete runs");
    assert_eq!(hashed, 1);
    let literal = ops
        .delete_keys(&[SecretString::from("literal-key")], None)
        .await
        .expect("the literal delete runs");
    assert_eq!(literal, 1);
    let absent = ops
        .delete_keys(&[SecretString::from("sk-absent")], None)
        .await
        .expect("the absent delete runs");
    assert_eq!(absent, 0);
    let remaining: i64 = db
        .query_one("SELECT count(*) FROM \"LiteLLM_VerificationToken\"", &[])
        .await
        .expect("the count runs")
        .get(0);
    assert_eq!(remaining, 0);
    harness.close().await;
}

#[tokio::test]
async fn delete_keys_for_teams_and_delete_teams_report_what_they_removed() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_VerificationToken\" (\"token\", \"team_id\", \"models\") VALUES ('t-1', 'team-a', ARRAY[]::text[]), ('t-2', 'team-a', ARRAY[]::text[]), ('t-3', 'team-b', ARRAY[]::text[])",
        &[],
    )
    .await
    .expect("the key rows commit");
    seed(&db, TEAM_INSERT, &[&"team-1"]).await;
    seed(&db, TEAM_INSERT, &[&"team-2"]).await;
    seed(&db, TEAM_INSERT, &[&"team-3"]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let removed = ops
        .delete_keys_for_teams(&[String::from("team-a")])
        .await
        .expect("the team key delete runs");
    assert_eq!(removed, 2);
    let teams = ops
        .delete_teams(&[String::from("team-1"), String::from("team-2")])
        .await
        .expect("the team delete runs");
    assert_eq!(teams, vec![String::from("team-1"), String::from("team-2")]);
    let remaining: i64 = db
        .query_one("SELECT count(*) FROM \"LiteLLM_TeamTable\"", &[])
        .await
        .expect("the count runs")
        .get(0);
    assert_eq!(remaining, 1);
    harness.close().await;
}
