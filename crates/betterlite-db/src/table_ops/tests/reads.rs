#![cfg(test)]
use crate::table_ops::generic::GenericReads as _;
use crate::table_ops::reads::configs::ConfigReads as _;
use crate::table_ops::reads::keys::KeyReads as _;
use crate::table_ops::reads::notifications::NotificationReads as _;
use crate::table_ops::reads::resets::ResetReads as _;
use crate::table_ops::reads::spend::SpendReads as _;
use crate::table_ops::reads::teams::TeamReads as _;
use crate::table_ops::reads::users::UserReads as _;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::Client;

use super::support::{Harness, Ports, TableClient, at, json, seed};
use crate::key_user_writes::hash::hash_token;
use crate::table_ops::generic::{ConfigFilter, KeyFilter};
use crate::table_ops::reads::spend::{SpendFilterColumn, SpendQuery};
use crate::table_ops::reads::users::UserFilter;

const USER_INSERT: &str = "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\", \"user_alias\", \"spend\") VALUES ($1, $2, $3)";
const KEY_INSERT: &str = "INSERT INTO \"LiteLLM_VerificationToken\" (\"token\", \"key_alias\", \"user_id\", \"spend\", \"models\") VALUES ($1, $2, $3, $4, ARRAY[]::text[])";
const TEAM_INSERT: &str = "INSERT INTO \"LiteLLM_TeamTable\" (\"team_id\", \"admins\", \"members\", \"models\") VALUES ($1, ARRAY[]::text[], ARRAY[]::text[], ARRAY[]::text[])";
const SPEND_INSERT: &str = "INSERT INTO \"LiteLLM_SpendLogs\" (\"request_id\", \"call_type\", \"startTime\", \"endTime\", \"user\", \"spend\") VALUES ($1, 'acompletion', $2, $2, $3, 1.0)";

async fn seed_spend(db: &Client, request_id: &str, user: &str, start_seconds: i64) {
    let start = moment(start_seconds).naive_utc();
    seed(db, SPEND_INSERT, &[&request_id, &start, &user]).await;
}

fn moment(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).expect("the timestamp is in range")
}

#[tokio::test]
async fn generic_user_reads_the_row_or_none() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, USER_INSERT, &[&"u-1", &"first", &2.0]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .generic_row(UserFilter::UserId("u-1"))
        .await
        .expect("the generic user read runs");
    assert_eq!(
        found.expect("the user row").user_alias.as_deref(),
        Some("first")
    );
    assert!(
        ops.generic_row(UserFilter::UserId("missing"))
            .await
            .expect("the generic miss runs")
            .is_none()
    );
    assert!(ports.sink.events().is_empty());
    harness.close().await;
}

#[tokio::test]
async fn generic_user_filters_on_a_caller_selected_column() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\", \"user_email\", \"sso_user_id\") VALUES ('u-mail', 'a@example.com', NULL), ('u-other', 'b@example.com', NULL)",
        &[],
    )
    .await
    .expect("the user rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let by_email = ops
        .generic_row(UserFilter::UserEmail("a@example.com"))
        .await
        .expect("the email read runs")
        .expect("the user row");
    assert_eq!(by_email.user_id, "u-mail");
    assert!(
        ops.generic_row(UserFilter::UserEmail("absent@example.com"))
            .await
            .expect("the email miss runs")
            .is_none()
    );
    harness.close().await;
}

#[tokio::test]
async fn generic_key_reads_the_stored_token() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, KEY_INSERT, &[&"stored-token", &"k-1", &"", &0.0]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .generic_row(KeyFilter::Token(&SecretString::from("stored-token")))
        .await
        .expect("the generic key read runs");
    assert_eq!(
        found.expect("the key row").key_alias.as_deref(),
        Some("k-1")
    );
    let by_alias = ops
        .generic_row(KeyFilter::KeyAlias("k-1"))
        .await
        .expect("the alias read runs")
        .expect("the key row");
    let stored = by_alias.token.as_ref().expect("the stored token");
    assert_eq!(stored.expose_secret(), "stored-token");
    assert!(
        ops.generic_row(KeyFilter::Token(&SecretString::from("other-token")))
            .await
            .expect("the generic miss runs")
            .is_none()
    );
    harness.close().await;
}

#[tokio::test]
async fn generic_config_decodes_json_text_and_object_values() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_Config\" (\"param_name\", \"param_value\") VALUES ($1, to_jsonb($2::text))",
        &[&"general_settings", &r#"{"store_model_in_db": true}"#],
    )
    .await
    .expect("the text config row commits");
    db.execute(
        "INSERT INTO \"LiteLLM_Config\" (\"param_name\", \"param_value\") VALUES ($1, $2::jsonb)",
        &[&"litellm_settings", &r#"{"num_retries": 3}"#],
    )
    .await
    .expect("the object config row commits");
    let run_at = at(1_700_000_000).utc().naive_utc();
    db.execute(
        "INSERT INTO \"LiteLLM_Config\" (\"param_name\", \"param_value\", \"reload_revision\", \"last_run_at\") VALUES ($1, NULL, $2, $3)",
        &[&"revision", &7_i64, &run_at],
    )
    .await
    .expect("the revision config row commits");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let text_row = ops
        .generic_row(ConfigFilter::ParamName("general_settings"))
        .await
        .expect("the text row read runs")
        .expect("the text row");
    assert_eq!(text_row.param_value, json(r#"{"store_model_in_db": true}"#));
    let object_row = ops
        .generic_row(ConfigFilter::ParamName("litellm_settings"))
        .await
        .expect("the object row read runs")
        .expect("the object row");
    assert_eq!(object_row.param_value, json(r#"{"num_retries": 3}"#));
    let revision_row = ops
        .generic_row(ConfigFilter::ReloadRevision(7))
        .await
        .expect("the revision read runs")
        .expect("the revision row");
    assert_eq!(revision_row.param_name, "revision");
    assert_eq!(revision_row.param_value, json("null"));
    let ran_row = ops
        .generic_row(ConfigFilter::LastRunAt(&at(1_700_000_000)))
        .await
        .expect("the last run read runs")
        .expect("the last run row");
    assert_eq!(ran_row.param_name, "revision");
    assert!(
        ops.generic_row(ConfigFilter::ParamName("missing"))
            .await
            .expect("the generic miss runs")
            .is_none()
    );
    harness.close().await;
}

#[tokio::test]
async fn configs_named_reads_only_the_named_rows() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_Config\" (\"param_name\", \"param_value\") VALUES ('a', '{\"n\":1}'::jsonb), ('b', '{\"n\":2}'::jsonb), ('c', '{\"n\":3}'::jsonb)",
        &[],
    )
    .await
    .expect("the config rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let rows = ops
        .configs_named(&[String::from("c"), String::from("a")])
        .await
        .expect("the named read runs");
    let mut names: Vec<String> = rows.into_iter().map(|row| row.param_name).collect();
    names.sort();
    assert_eq!(names, vec![String::from("a"), String::from("c")]);
    harness.close().await;
}

#[tokio::test]
async fn keys_for_user_reads_only_the_users_keys() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, KEY_INSERT, &[&"tok-1", &"k-1", &"u-1", &1.0]).await;
    seed(&db, KEY_INSERT, &[&"tok-2", &"k-2", &"u-1", &2.0]).await;
    seed(&db, KEY_INSERT, &[&"tok-3", &"k-3", &"u-2", &3.0]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .keys_for_user("u-1")
        .await
        .expect("the user key read runs");
    let mut aliases: Vec<Option<String>> = found.into_iter().map(|key| key.key_alias).collect();
    aliases.sort();
    assert_eq!(
        aliases,
        vec![Some(String::from("k-1")), Some(String::from("k-2"))]
    );
    harness.close().await;
}

#[tokio::test]
async fn keys_matching_hashes_plaintext_tokens() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(
        &db,
        KEY_INSERT,
        &[&hash_token("sk-abc"), &"hashed", &"", &0.0],
    )
    .await;
    seed(&db, KEY_INSERT, &[&"plain", &"literal", &"", &0.0]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let tokens = [SecretString::from("sk-abc"), SecretString::from("plain")];
    let found = ops
        .keys_matching(Some(&tokens))
        .await
        .expect("the matching read runs");
    let mut aliases: Vec<Option<String>> = found.into_iter().map(|key| key.key_alias).collect();
    aliases.sort();
    assert_eq!(
        aliases,
        vec![Some(String::from("hashed")), Some(String::from("literal"))]
    );
    let absent = [SecretString::from("absent")];
    assert!(
        ops.keys_matching(Some(&absent))
            .await
            .expect("the empty match runs")
            .is_empty()
    );
    harness.close().await;
}

#[tokio::test]
async fn keys_matching_without_tokens_orders_by_spend() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, KEY_INSERT, &[&"tok-low", &"low", &"", &1.0]).await;
    seed(&db, KEY_INSERT, &[&"tok-high", &"high", &"", &9.0]).await;
    seed(&db, KEY_INSERT, &[&"tok-mid", &"mid", &"", &4.0]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .keys_matching(None)
        .await
        .expect("the unfiltered read runs");
    let aliases: Vec<Option<String>> = found.into_iter().map(|key| key.key_alias).collect();
    assert_eq!(
        aliases,
        vec![
            Some(String::from("high")),
            Some(String::from("mid")),
            Some(String::from("low")),
        ]
    );
    harness.close().await;
}

#[tokio::test]
async fn keys_for_team_honours_the_limit() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_VerificationToken\" (\"token\", \"key_alias\", \"team_id\", \"models\") VALUES ('t-1', 'k-1', 'team-a', ARRAY[]::text[]), ('t-2', 'k-2', 'team-a', ARRAY[]::text[]), ('t-3', 'k-3', 'team-a', ARRAY[]::text[]), ('t-4', 'k-4', 'team-b', ARRAY[]::text[])",
        &[],
    )
    .await
    .expect("the team key rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let limited = ops
        .keys_for_team("team-a", Some(2))
        .await
        .expect("the limited read runs");
    assert_eq!(limited.len(), 2);
    let all = ops
        .keys_for_team("team-a", None)
        .await
        .expect("the unlimited read runs");
    assert_eq!(all.len(), 3);
    harness.close().await;
}

#[tokio::test]
async fn keys_due_for_reset_keeps_live_keys_past_their_reset() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_VerificationToken\" (\"token\", \"key_alias\", \"expires\", \"budget_reset_at\", \"budget_duration\", \"models\") VALUES
         ('k1', 'live', NULL, '2020-01-01'::timestamp, '1d', ARRAY[]::text[]),
         ('k2', 'expired', '2000-01-01'::timestamp, '2020-01-01'::timestamp, '1d', ARRAY[]::text[]),
         ('k3', 'future', '2100-01-01'::timestamp, '2020-01-01'::timestamp, '1d', ARRAY[]::text[]),
         ('k4', 'undated', NULL, '2020-01-01'::timestamp, NULL, ARRAY[]::text[]),
         ('k5', 'settled', NULL, '2100-01-01'::timestamp, '1d', ARRAY[]::text[])",
        &[],
    )
    .await
    .expect("the reset key rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .keys_due_for_reset(moment(1_000_000_000), moment(2_000_000_000), None)
        .await
        .expect("the reset read runs");
    let mut aliases: Vec<Option<String>> = found.into_iter().map(|key| key.key_alias).collect();
    aliases.sort();
    assert_eq!(
        aliases,
        vec![Some(String::from("future")), Some(String::from("live"))]
    );
    let limited = ops
        .keys_due_for_reset(moment(1_000_000_000), moment(2_000_000_000), Some(1))
        .await
        .expect("the limited reset read runs");
    assert_eq!(limited.len(), 1);
    harness.close().await;
}

#[tokio::test]
async fn users_matching_reads_each_identity_column() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\", \"user_alias\", \"user_email\", \"sso_user_id\") VALUES ('u-mail', 'mail', 'a@example.com', NULL), ('u-sso', 'sso', NULL, 'sso-1')",
        &[],
    )
    .await
    .expect("the user rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let by_mail = ops
        .users_matching(UserFilter::UserEmail("a@example.com"))
        .await
        .expect("the email read runs");
    assert_eq!(by_mail.len(), 1);
    assert_eq!(by_mail[0].user_id, "u-mail");
    let by_sso = ops
        .users_matching(UserFilter::SsoUserId("sso-1"))
        .await
        .expect("the sso read runs");
    assert_eq!(by_sso[0].user_id, "u-sso");
    assert!(
        ops.users_matching(UserFilter::UserId("absent"))
            .await
            .expect("the empty read runs")
            .is_empty()
    );
    harness.close().await;
}

#[tokio::test]
async fn user_unique_returns_organization_memberships() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\", \"created_by\", \"updated_by\") VALUES ('org-budget', 'c', 'u')",
        &[],
    )
    .await
    .expect("the budget row commits");
    db.execute(
        "INSERT INTO \"LiteLLM_OrganizationTable\" (\"organization_id\", \"organization_alias\", \"budget_id\", \"created_by\", \"updated_by\") VALUES ('org-1', 'acme', 'org-budget', 'c', 'u')",
        &[],
    )
    .await
    .expect("the organization row commits");
    seed(&db, USER_INSERT, &[&"u-1", &"member", &0.0]).await;
    db.execute(
        "INSERT INTO \"LiteLLM_OrganizationMembership\" (\"user_id\", \"organization_id\") VALUES ('u-1', 'org-1')",
        &[],
    )
    .await
    .expect("the membership row commits");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .user_unique("u-1")
        .await
        .expect("the unique read runs")
        .expect("the user row");
    assert_eq!(found.user_alias.as_deref(), Some("member"));
    assert_eq!(
        found
            .organization_memberships
            .expect("the memberships")
            .len(),
        1
    );
    assert!(
        ops.user_unique("absent")
            .await
            .expect("the unique miss runs")
            .is_none()
    );
    harness.close().await;
}

#[tokio::test]
async fn users_due_for_reset_keeps_budget_durations() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\", \"user_alias\", \"budget_reset_at\", \"budget_duration\") VALUES
         ('u1', 'pending', '2020-01-01'::timestamp, '1d'),
         ('u2', 'null-reset', NULL, '1d'),
         ('u3', 'no-duration', '2020-01-01'::timestamp, NULL),
         ('u4', 'settled', '2100-01-01'::timestamp, '1d')",
        &[],
    )
    .await
    .expect("the reset user rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .users_due_for_reset(moment(2_000_000_000), None)
        .await
        .expect("the reset read runs");
    let mut aliases: Vec<Option<String>> = found.into_iter().map(|user| user.user_alias).collect();
    aliases.sort();
    assert_eq!(
        aliases,
        vec![
            Some(String::from("null-reset")),
            Some(String::from("pending")),
        ]
    );
    let limited = ops
        .users_due_for_reset(moment(2_000_000_000), Some(1))
        .await
        .expect("the limited reset read runs");
    assert_eq!(limited.len(), 1);
    harness.close().await;
}

#[tokio::test]
async fn users_by_ids_returns_each_requested_row() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, USER_INSERT, &[&"u-1", &"one", &0.0]).await;
    seed(&db, USER_INSERT, &[&"u-2", &"two", &0.0]).await;
    seed(&db, USER_INSERT, &[&"u-3", &"three", &0.0]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .users_by_ids(&[String::from("u-3"), String::from("u-1")])
        .await
        .expect("the ids read runs");
    let mut ids: Vec<String> = found.into_iter().map(|user| user.user_id).collect();
    ids.sort();
    assert_eq!(ids, vec![String::from("u-1"), String::from("u-3")]);
    harness.close().await;
}

#[tokio::test]
async fn users_all_joins_key_aliases_and_orders_by_spend() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, USER_INSERT, &[&"u-high", &"high", &9.0]).await;
    seed(&db, USER_INSERT, &[&"u-low", &"low", &1.0]).await;
    seed(&db, KEY_INSERT, &[&"tok-1", &"alias-1", &"u-high", &0.0]).await;
    seed(&db, KEY_INSERT, &[&"tok-2", &"alias-2", &"u-high", &0.0]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops.users_all(None, None).await.expect("the list read runs");
    assert_eq!(found[0].user.user_id, "u-high");
    assert_eq!(found[1].user.user_id, "u-low");
    assert_eq!(found[0].key_aliases.len(), 2);
    let second = ops
        .users_all(Some(1), Some(1))
        .await
        .expect("the paged read runs");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].user.user_id, "u-low");
    harness.close().await;
}

#[tokio::test]
async fn spend_rows_read_one_or_many_rows_by_column() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed_spend(&db, "req-1", "u-1", 0).await;
    seed_spend(&db, "req-2", "u-1", 60).await;
    seed_spend(&db, "req-3", "u-2", 30).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let unique = ops
        .spend_rows(SpendQuery::UNIQUE, SpendFilterColumn::User, "u-1")
        .await
        .expect("the unique spend read runs");
    assert_eq!(unique.len(), 1);
    let matching = ops
        .spend_rows(SpendQuery::MATCHING, SpendFilterColumn::User, "u-1")
        .await
        .expect("the matching spend read runs");
    assert_eq!(matching.len(), 2);
    let by_request = ops
        .spend_rows(SpendQuery::UNIQUE, SpendFilterColumn::RequestId, "req-3")
        .await
        .expect("the request spend read runs");
    assert_eq!(by_request[0].user.as_deref(), Some("u-2"));
    harness.close().await;
}

#[tokio::test]
async fn spend_all_orders_by_start_time_descending() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed_spend(&db, "req-early", "u-1", 0).await;
    seed_spend(&db, "req-late", "u-1", 120).await;
    seed_spend(&db, "req-mid", "u-1", 60).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops.spend_all().await.expect("the spend list runs");
    let ids: Vec<String> = found.into_iter().map(|row| row.request_id).collect();
    assert_eq!(
        ids,
        vec![
            String::from("req-late"),
            String::from("req-mid"),
            String::from("req-early"),
        ]
    );
    harness.close().await;
}

#[tokio::test]
async fn spend_rows_reads_the_first_row_generically() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed_spend(&db, "req-1", "u-1", 0).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .generic_spend(SpendFilterColumn::RequestId, "req-1")
        .await
        .expect("the generic spend read runs")
        .expect("the spend row");
    assert_eq!(found.request_id, "req-1");
    assert_eq!(found.call_type, "acompletion");
    let by_user = ops
        .generic_spend(SpendFilterColumn::User, "u-1")
        .await
        .expect("the generic user spend read runs")
        .expect("the spend row");
    assert_eq!(by_user.request_id, "req-1");
    assert!(
        ops.generic_spend(SpendFilterColumn::RequestId, "absent")
            .await
            .expect("the generic miss runs")
            .is_none()
    );
    harness.close().await;
}

#[tokio::test]
async fn budgets_due_for_reset_filters_on_duration_and_reset_at() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\", \"created_by\", \"updated_by\", \"budget_duration\", \"budget_reset_at\") VALUES
         ('b1', 'c', 'u', '1d', '2020-01-01'::timestamp),
         ('b2', 'c', 'u', '1d', NULL),
         ('b3', 'c', 'u', NULL, '2020-01-01'::timestamp),
         ('b4', 'c', 'u', '1d', '2100-01-01'::timestamp)",
        &[],
    )
    .await
    .expect("the budget rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .budgets_due_for_reset(moment(2_000_000_000), None)
        .await
        .expect("the budget reset read runs");
    let mut ids: Vec<Option<String>> = found.into_iter().map(|row| row.budget.budget_id).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec![Some(String::from("b1")), Some(String::from("b2"))]
    );
    let limited = ops
        .budgets_due_for_reset(moment(2_000_000_000), Some(1))
        .await
        .expect("the limited reset read runs");
    assert_eq!(limited.len(), 1);
    harness.close().await;
}

#[tokio::test]
async fn end_users_by_budgets_returns_the_linked_rows() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_BudgetTable\" (\"budget_id\", \"created_by\", \"updated_by\") VALUES ('b1', 'c', 'u'), ('b2', 'c', 'u'), ('b3', 'c', 'u')",
        &[],
    )
    .await
    .expect("the budget rows commit");
    db.execute(
        "INSERT INTO \"LiteLLM_EndUserTable\" (\"user_id\", \"budget_id\") VALUES ('e1', 'b1'), ('e2', 'b2'), ('e3', 'b3')",
        &[],
    )
    .await
    .expect("the end user rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .end_users_by_budgets(&[String::from("b3"), String::from("b1")])
        .await
        .expect("the end user read runs");
    let mut ids: Vec<String> = found.into_iter().map(|row| row.user_id).collect();
    ids.sort();
    assert_eq!(ids, vec![String::from("e1"), String::from("e3")]);
    harness.close().await;
}

#[tokio::test]
async fn team_unique_includes_the_model_table() {
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
    db.execute(
        "INSERT INTO \"LiteLLM_TeamTable\" (\"team_id\", \"team_alias\", \"admins\", \"members\", \"models\", \"model_id\") VALUES ('team-1', 'acme', ARRAY[]::text[], ARRAY[]::text[], ARRAY[]::text[], $1)",
        &[&model_id],
    )
    .await
    .expect("the team row commits");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .team_unique("team-1")
        .await
        .expect("the team read runs")
        .expect("the team row");
    assert_eq!(found.fields.base.team_alias.as_deref(), Some("acme"));
    assert!(found.fields.litellm_model_table.is_some());
    assert!(
        ops.team_unique("absent")
            .await
            .expect("the team miss runs")
            .is_none()
    );
    harness.close().await;
}

#[tokio::test]
async fn teams_for_user_reads_the_members_array() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_TeamTable\" (\"team_id\", \"admins\", \"members\", \"models\") VALUES
         ('team-1', ARRAY[]::text[], ARRAY['u-1']::text[], ARRAY[]::text[]),
         ('team-2', ARRAY[]::text[], ARRAY['u-2']::text[], ARRAY[]::text[])",
        &[],
    )
    .await
    .expect("the team rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .teams_for_user("u-1")
        .await
        .expect("the member read runs");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].fields.base.team_id, "team-1");
    harness.close().await;
}

#[tokio::test]
async fn teams_by_ids_returns_the_requested_teams() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    seed(&db, TEAM_INSERT, &[&"team-1"]).await;
    seed(&db, TEAM_INSERT, &[&"team-2"]).await;
    seed(&db, TEAM_INSERT, &[&"team-3"]).await;
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .teams_by_ids(&[String::from("team-1"), String::from("team-3")])
        .await
        .expect("the ids read runs");
    assert_eq!(found.len(), 2);
    harness.close().await;
}

#[tokio::test]
async fn teams_all_stops_at_the_twenty_row_default() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_TeamTable\" (\"team_id\", \"admins\", \"members\", \"models\") SELECT 'team-' || generated, ARRAY[]::text[], ARRAY[]::text[], ARRAY[]::text[] FROM generate_series(1, 25) AS generated",
        &[],
    )
    .await
    .expect("the team rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops.teams_all().await.expect("the team list runs");
    assert_eq!(found.len(), 20);
    harness.close().await;
}

#[tokio::test]
async fn teams_due_for_reset_filters_on_duration_and_reset_at() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_TeamTable\" (\"team_id\", \"admins\", \"members\", \"models\", \"budget_duration\", \"budget_reset_at\") VALUES
         ('t1', ARRAY[]::text[], ARRAY[]::text[], ARRAY[]::text[], '1d', '2020-01-01'::timestamp),
         ('t2', ARRAY[]::text[], ARRAY[]::text[], ARRAY[]::text[], '1d', NULL),
         ('t3', ARRAY[]::text[], ARRAY[]::text[], ARRAY[]::text[], NULL, '2020-01-01'::timestamp),
         ('t4', ARRAY[]::text[], ARRAY[]::text[], ARRAY[]::text[], '1d', '2100-01-01'::timestamp)",
        &[],
    )
    .await
    .expect("the team rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .teams_due_for_reset(moment(2_000_000_000), None)
        .await
        .expect("the team reset read runs");
    let mut ids: Vec<String> = found
        .into_iter()
        .map(|team| team.fields.base.team_id)
        .collect();
    ids.sort();
    assert_eq!(ids, vec![String::from("t1"), String::from("t2")]);
    harness.close().await;
}

#[tokio::test]
async fn user_notifications_read_the_first_row_and_all_rows() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_UserNotifications\" (\"request_id\", \"user_id\", \"models\", \"justification\", \"status\") VALUES
         ('r-1', 'u-1', ARRAY[]::text[], 'first', 'pending'),
         ('r-2', 'u-1', ARRAY[]::text[], 'second', 'pending'),
         ('r-3', 'u-2', ARRAY['gpt-4']::text[], 'other', 'pending'),
         ('r-4', 'u-3', NULL, 'unmodelled', 'pending')",
        &[],
    )
    .await
    .expect("the notification rows commit");
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    let found = ops
        .user_notification_unique("u-1")
        .await
        .expect("the unique read runs")
        .expect("the first row");
    assert_eq!(found.user_id, "u-1");
    assert!(found.request_id == "r-1" || found.request_id == "r-2");
    let all = ops
        .user_notifications_all()
        .await
        .expect("the notification list runs");
    assert_eq!(all.len(), 4);
    let unmodelled = all
        .iter()
        .find(|row| row.request_id == "r-4")
        .expect("the null-models row");
    assert_eq!(unmodelled.models, None);
    assert!(
        ops.user_notification_unique("absent")
            .await
            .expect("the unique miss runs")
            .is_none()
    );
    harness.close().await;
}

#[tokio::test]
async fn generic_key_boolean_filter_binds_true_and_false() {
    let harness = Harness::start().await;
    let db = harness.pool.get().await.unwrap();
    db.batch_execute("INSERT INTO \"LiteLLM_VerificationToken\" (token, blocked) VALUES ('blocked-key', true), ('active-key', false)").await.unwrap();
    let client = TableClient::new(harness.pool.clone());
    let ports = Ports::new();
    let ops = ports.operations(&client);
    for (flag, token) in [(true, "blocked-key"), (false, "active-key")] {
        let row = ops
            .generic_row(KeyFilter::Blocked(flag))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.token.expose_secret(), token);
    }
    harness.close().await;
}
