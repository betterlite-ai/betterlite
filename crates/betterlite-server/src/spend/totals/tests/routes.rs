use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use rstest::rstest;
use rust_decimal::Decimal;

use betterlite_auth::auth_identity::roles::Role;
use betterlite_config::spend_storage::load_spend_storage_policy;
use betterlite_logging::prometheus_query::daily_spend::PrometheusDailySpend;

use crate::auth::auth_key_lookup::tests::caller_through_key_stage;

use super::super::rows::{
    DailySpend, KeyDailySpend, KeySpend, ModelSpend, TeamSpend, TotalSpendRow, UserKeyDailySpend,
    UserModelSpend,
};
use super::support::{
    DailyBody, DetailMessageEnvelope, ErrorEnvelope, FakeFailure, FakePrometheus, FakeStore,
    InternalEnvelope, KeyBody, KeyDailyBody, ModelBody, TeamsBody, TotalBody, UserKeyDailyBody,
    UserModelBody, ValidationEnvelope, admin, body_text, call, date, decimal, decode, internal,
    internal_without_user, master_key, prometheus, role_holder, spend_store, state,
};

fn daily(day: &str, spend: &str) -> DailySpend {
    DailySpend {
        date: date(day),
        spend: decimal(spend),
    }
}

fn key_daily(day: &str, spend: &str, api_key: &str) -> KeyDailySpend {
    KeyDailySpend {
        date: date(day),
        spend: decimal(spend),
        api_key: api_key.to_owned(),
    }
}

fn user_daily(day: &str, spend: &str, api_key: &str, user: &str) -> UserKeyDailySpend {
    UserKeyDailySpend {
        date: date(day),
        spend: decimal(spend),
        api_key: api_key.to_owned(),
        user: user.to_owned(),
    }
}

fn key_spend(api_key: &str, spend: &str) -> KeySpend {
    KeySpend {
        api_key: api_key.to_owned(),
        total_spend: decimal(spend),
        key_alias: None,
        key_name: None,
    }
}

fn team(alias: Option<&str>, day: &str, spend: Option<&str>) -> TeamSpend {
    TeamSpend {
        team_alias: alias.map(str::to_owned),
        spend_date: Some(date(day)),
        total_spend: spend.map(decimal),
    }
}

fn total(spend: Option<&str>) -> TotalSpendRow {
    TotalSpendRow {
        total_spend: spend.map(decimal),
    }
}

fn prometheus_row(day: &str, spend: &str) -> PrometheusDailySpend {
    PrometheusDailySpend {
        date: day.to_owned(),
        spend: decimal(spend),
    }
}

fn store_with(rows: FakeStore) -> Arc<FakeStore> {
    Arc::new(rows)
}

fn source_with(rows: FakePrometheus) -> Arc<FakePrometheus> {
    Arc::new(rows)
}

#[rstest]
#[case::logs("/global/spend/logs")]
#[case::spend("/global/spend")]
#[case::keys("/global/spend/keys")]
#[case::models("/global/spend/models")]
#[case::teams("/global/spend/teams")]
#[tokio::test]
async fn refuses_a_request_without_an_http_identity(#[case] uri: &str) {
    let reply = call(uri, None, state(None, None, Decimal::ZERO)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let body: ErrorEnvelope = decode(&reply);
    assert_eq!(body.error.message, "the API key is not valid");
    assert_eq!(body.error.kind, "auth_error");
    assert_eq!(body.error.param, "None");
    assert_eq!(body.error.code, "401");
}

#[tokio::test]
async fn logs_read_the_user_key_view_for_an_internal_viewer() {
    let mut fake = spend_store();
    fake.user_daily = vec![user_daily("2026-01-01", "1.25", "sk-one", "u1")];
    let store = store_with(fake);
    let reply = call(
        "/global/spend/logs?api_key=sk-one",
        Some(&internal("u1")),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Vec<UserKeyDailyBody> = decode(&reply);
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].date, "2026-01-01");
    assert_eq!(body[0].user, "u1");
    assert_eq!(body[0].api_key, "sk-one");
    assert_eq!(body[0].spend, decimal("1.25"));
    assert_eq!(
        store.calls(),
        vec!["user_daily_spend(u1,Some(\"sk-one\"))".to_owned()]
    );
}

#[tokio::test]
async fn logs_scope_a_user_bound_key_to_its_user() {
    let mut fake = spend_store();
    fake.user_daily = vec![user_daily("2026-01-01", "1.25", "sk-one", "u1")];
    let store = store_with(fake);
    let caller = caller_through_key_stage(r#"{"user_id":"u1","user_role":"internal_user"}"#);
    let reply = call(
        "/global/spend/logs?api_key=sk-one",
        Some(&caller.principal),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec!["user_daily_spend(u1,Some(\"sk-one\"))".to_owned()]
    );
}

#[tokio::test]
async fn logs_scope_an_internal_viewer_without_a_key_to_their_user() {
    let mut fake = spend_store();
    fake.user_daily = vec![
        user_daily("2026-01-01", "1.25", "sk-one", "u1"),
        user_daily("2026-01-01", "2.00", "sk-two", "u1"),
    ];
    let store = store_with(fake);
    let reply = call(
        "/global/spend/logs",
        Some(&internal("u1")),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Vec<UserKeyDailyBody> = decode(&reply);
    assert_eq!(body.len(), 2);
    assert_eq!(store.calls(), vec!["user_daily_spend(u1,None)".to_owned()]);
}

#[tokio::test]
async fn logs_wrap_a_missing_internal_user_id() {
    let store = store_with(spend_store());
    let reply = call(
        "/global/spend/logs",
        Some(&internal_without_user()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: ErrorEnvelope = decode(&reply);
    assert_eq!(
        body.error.message,
        "/global/spend/logs Error: User ID is None"
    );
    assert_eq!(body.error.kind, "internal_error");
    assert_eq!(body.error.param, "None");
    assert_eq!(body.error.code, "500");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn logs_require_a_database_even_when_prometheus_is_connected() {
    let mut fake_source = prometheus();
    fake_source.rows = vec![prometheus_row("2026-01-01T00:00:00+00:00", "9")];
    let source = source_with(fake_source);
    let reply = call(
        "/global/spend/logs",
        Some(&admin()),
        state(None, Some(&source), Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: ErrorEnvelope = decode(&reply);
    assert_eq!(body.error.message, "Prisma Client is not initialized");
    assert_eq!(body.error.kind, "internal_error");
    assert_eq!(body.error.param, "None");
    assert_eq!(body.error.code, "500");
    assert!(source.calls().is_empty());
}

#[tokio::test]
async fn logs_use_prometheus_for_a_global_caller() {
    let mut fake = spend_store();
    fake.global_daily = vec![daily("2026-01-01", "1")];
    let store = store_with(fake);
    let mut fake_source = prometheus();
    fake_source.rows = vec![prometheus_row("2026-01-01T00:00:00+00:00", "9.5")];
    let source = source_with(fake_source);
    let reply = call(
        "/global/spend/logs?api_key=sk-one",
        Some(&admin()),
        state(Some(&store), Some(&source), Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Vec<DailyBody> = decode(&reply);
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].date, "2026-01-01T00:00:00+00:00");
    assert_eq!(body[0].spend, decimal("9.5"));
    assert!(store.calls().is_empty());
    assert_eq!(source.calls(), vec!["daily_spend(sk-one)".to_owned()]);
}

#[tokio::test]
async fn logs_keep_an_internal_viewer_on_the_database_with_prometheus_connected() {
    let mut fake = spend_store();
    fake.user_daily = vec![user_daily("2026-01-01", "1.25", "sk-one", "u1")];
    let store = store_with(fake);
    let mut fake_source = prometheus();
    fake_source.rows = vec![prometheus_row("2026-01-01T00:00:00+00:00", "9.5")];
    let source = source_with(fake_source);
    let reply = call(
        "/global/spend/logs",
        Some(&internal("u1")),
        state(Some(&store), Some(&source), Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Vec<UserKeyDailyBody> = decode(&reply);
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].spend, decimal("1.25"));
    assert!(source.calls().is_empty());
    assert_eq!(store.calls(), vec!["user_daily_spend(u1,None)".to_owned()]);
}

#[tokio::test]
async fn logs_read_the_monthly_view_without_prometheus() {
    let mut fake = spend_store();
    fake.global_daily = vec![daily("2026-01-01", "1"), daily("2026-01-02", "2.5")];
    let store = store_with(fake);
    let reply = call(
        "/global/spend/logs",
        Some(&master_key()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Vec<DailyBody> = decode(&reply);
    assert_eq!(body.len(), 2);
    assert_eq!(body[1].spend, decimal("2.5"));
    assert_eq!(store.calls(), vec!["global_daily_spend".to_owned()]);
}

#[rstest]
#[case::platform_admin(Role::PlatformAdmin, "global_daily_spend")]
#[case::platform_viewer(Role::PlatformViewer, "global_daily_spend")]
#[case::org_admin(Role::OrgAdmin, "global_daily_spend")]
#[case::org_viewer(Role::OrgViewer, "user_daily_spend(u1,None)")]
#[case::team_admin(Role::TeamAdmin, "user_daily_spend(u1,None)")]
#[case::team_member(Role::TeamMember, "user_daily_spend(u1,None)")]
#[tokio::test]
async fn logs_route_each_role_to_its_scope(#[case] role: Role, #[case] expected: &str) {
    let store = store_with(spend_store());
    let reply = call(
        "/global/spend/logs",
        Some(&role_holder(role, "u1")),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec![expected.to_owned()]);
}

#[tokio::test]
async fn logs_scope_the_monthly_view_to_the_requested_key() {
    let mut fake = spend_store();
    fake.key_daily = vec![key_daily("2026-01-01", "3.5", "sk-one")];
    let store = store_with(fake);
    let reply = call(
        "/global/spend/logs?api_key=sk-one",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Vec<KeyDailyBody> = decode(&reply);
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].date, "2026-01-01");
    assert_eq!(body[0].api_key, "sk-one");
    assert_eq!(body[0].spend, decimal("3.5"));
    assert_eq!(store.calls(), vec!["key_daily_spend(sk-one)".to_owned()]);
}

#[tokio::test]
async fn logs_wrap_a_prometheus_failure_without_falling_back() {
    let store = store_with(spend_store());
    let mut fake_source = prometheus();
    fake_source.fails = true;
    let source = source_with(fake_source);
    let reply = call(
        "/global/spend/logs",
        Some(&admin()),
        state(Some(&store), Some(&source), Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: ErrorEnvelope = decode(&reply);
    assert_eq!(body.error.message, "/global/spend/logs Error");
    assert_eq!(body.error.kind, "internal_error");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn logs_wrap_a_database_failure() {
    let mut fake = spend_store();
    fake.failure = Some(FakeFailure::InvalidAmount);
    let store = store_with(fake);
    let reply = call(
        "/global/spend/logs",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: ErrorEnvelope = decode(&reply);
    assert!(body.error.message.starts_with("/global/spend/logs Error: "));
    assert_eq!(body.error.kind, "internal_error");
    assert_eq!(body.error.code, "500");
}

#[tokio::test]
async fn logs_wrap_an_internal_database_failure() {
    let mut fake = spend_store();
    fake.failure = Some(FakeFailure::InvalidAmount);
    let store = store_with(fake);
    let reply = call(
        "/global/spend/logs",
        Some(&internal("u1")),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: ErrorEnvelope = decode(&reply);
    assert!(body.error.message.starts_with("/global/spend/logs Error: "));
    assert_eq!(body.error.kind, "internal_error");
    assert_eq!(body.error.code, "500");
}

#[tokio::test]
async fn spend_sums_the_monthly_view_for_an_internal_caller() {
    let mut fake = spend_store();
    fake.total = vec![total(Some("4.5"))];
    let store = store_with(fake);
    let reply = call(
        "/global/spend",
        Some(&internal("u1")),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: TotalBody = decode(&reply);
    assert_eq!(body.spend, Some(decimal("4.5")));
    assert_eq!(body.max_budget, Decimal::ZERO);
    assert!(body_text(&reply).contains(r#""spend":4.5"#));
    assert_eq!(store.calls(), vec!["global_total_spend".to_owned()]);
}

#[tokio::test]
async fn spend_starts_at_zero_without_a_result_row() {
    let store = store_with(spend_store());
    let reply = call(
        "/global/spend",
        Some(&master_key()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: TotalBody = decode(&reply);
    assert_eq!(body.spend, Some(Decimal::ZERO));
    assert!(body_text(&reply).contains(r#""spend":0"#));
}

#[tokio::test]
async fn spend_keeps_a_null_sum_null() {
    let mut fake = spend_store();
    fake.total = vec![total(None)];
    let store = store_with(fake);
    let reply = call(
        "/global/spend",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: TotalBody = decode(&reply);
    assert_eq!(body.spend, None);
    assert!(body_text(&reply).contains(r#""spend":null"#));
}

#[rstest]
#[case::logs("/global/spend/logs")]
#[case::spend("/global/spend")]
#[case::teams("/global/spend/teams")]
#[tokio::test]
async fn routes_without_a_limit_parameter_ignore_unparsable_text(#[case] uri: &str) {
    let store = store_with(spend_store());
    let reply = call(
        &format!("{uri}?limit=five"),
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn spend_requires_a_database() {
    let reply = call(
        "/global/spend",
        Some(&admin()),
        state(None, None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: ErrorEnvelope = decode(&reply);
    assert_eq!(body.error.message, "No db connected");
    assert_eq!(body.error.kind, "internal_error");
    assert_eq!(body.error.param, "None");
    assert_eq!(body.error.code, "500");
}

#[tokio::test]
async fn spend_serves_the_configured_max_budget() {
    let settings = load_spend_storage_policy(
        "litellm_settings:\n  max_budget: 42.5\n",
        &BTreeMap::<String, String>::new(),
    )
    .unwrap();
    let mut fake = spend_store();
    fake.total = vec![total(Some("7"))];
    let store = store_with(fake);
    let reply = call(
        "/global/spend",
        Some(&admin()),
        state(Some(&store), None, settings.max_budget),
    )
    .await;
    let body: TotalBody = decode(&reply);
    assert_eq!(body.spend, Some(decimal("7")));
    assert_eq!(body.max_budget, decimal("42.5"));
    assert!(body_text(&reply).contains(r#""max_budget":42.5"#));
}

#[tokio::test]
async fn spend_wraps_a_database_failure() {
    let mut fake = spend_store();
    fake.failure = Some(FakeFailure::InvalidAmount);
    let store = store_with(fake);
    let reply = call(
        "/global/spend",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: ErrorEnvelope = decode(&reply);
    assert!(body.error.message.starts_with("/global/spend Error: "));
    assert_eq!(body.error.kind, "internal_error");
    assert_eq!(body.error.code, "500");
}

#[tokio::test]
async fn keys_read_the_last_thirty_days_view_without_a_limit() {
    let mut fake = spend_store();
    fake.keys = vec![
        KeySpend {
            api_key: "sk-one".to_owned(),
            total_spend: decimal("1.5"),
            key_alias: Some("primary".to_owned()),
            key_name: Some("one".to_owned()),
        },
        key_spend("sk-two", "0.5"),
    ];
    let store = store_with(fake);
    let reply = call(
        "/global/spend/keys",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Vec<KeyBody> = decode(&reply);
    assert_eq!(body.len(), 2);
    assert_eq!(body[0].api_key, "sk-one");
    assert_eq!(body[0].total_spend, decimal("1.5"));
    assert_eq!(body[0].key_alias.as_deref(), Some("primary"));
    assert_eq!(body[0].key_name.as_deref(), Some("one"));
    assert_eq!(body[1].key_alias, None);
    assert_eq!(body[1].key_name, None);
    assert_eq!(store.calls(), vec!["keys_by_spend(None)".to_owned()]);
}

#[rstest]
#[case::smallest_positive(1, "keys_by_spend(Some(1))")]
#[case::larger(5, "keys_by_spend(Some(5))")]
#[tokio::test]
async fn keys_pass_a_positive_limit(#[case] limit: i64, #[case] expected: &str) {
    let mut fake = spend_store();
    fake.keys = vec![key_spend("sk-one", "1.5")];
    let store = store_with(fake);
    let reply = call(
        &format!("/global/spend/keys?limit={limit}"),
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec![expected.to_owned()]);
}

#[rstest]
#[case::zero(
    "/global/spend/keys?limit=0",
    "Invalid limit: 0, error: Limit must be greater than 0"
)]
#[case::negative(
    "/global/spend/keys?limit=-3",
    "Invalid limit: -3, error: Limit must be greater than 0"
)]
#[tokio::test]
async fn keys_reject_a_nonpositive_limit(#[case] uri: &str, #[case] message: &str) {
    let store = store_with(spend_store());
    let reply = call(
        uri,
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body: DetailMessageEnvelope = decode(&reply);
    assert_eq!(body.detail.error, message);
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn keys_reject_text_that_is_not_an_integer() {
    let store = store_with(spend_store());
    let reply = call(
        "/global/spend/keys?limit=five",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body: ValidationEnvelope = decode(&reply);
    assert_eq!(body.detail.len(), 1);
    assert_eq!(body.detail[0].kind, "int_parsing");
    assert_eq!(body.detail[0].loc, ["query", "limit"]);
    assert_eq!(
        body.detail[0].msg,
        "Input should be a valid integer, unable to parse string as an integer"
    );
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn keys_ignore_the_query_limit_for_an_internal_viewer() {
    let mut fake = spend_store();
    fake.user_keys = vec![key_spend("sk-one", "1.5")];
    let store = store_with(fake);
    let reply = call(
        "/global/spend/keys?limit=0",
        Some(&internal("u1")),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Vec<KeyBody> = decode(&reply);
    assert_eq!(body.len(), 1);
    assert_eq!(store.calls(), vec!["user_keys_by_spend(u1,10)".to_owned()]);
}

#[tokio::test]
async fn keys_scope_a_user_bound_key_to_its_user() {
    let mut fake = spend_store();
    fake.user_keys = vec![key_spend("sk-one", "1.5")];
    let store = store_with(fake);
    let caller = caller_through_key_stage(r#"{"user_id":"u1","user_role":"internal_user"}"#);
    let reply = call(
        "/global/spend/keys",
        Some(&caller.principal),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec!["user_keys_by_spend(u1,10)".to_owned()]);
}

#[tokio::test]
async fn keys_reject_an_invalid_limit_for_an_internal_viewer() {
    let store = store_with(spend_store());
    let reply = call(
        "/global/spend/keys?limit=five",
        Some(&internal("u1")),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn keys_report_a_missing_internal_user_id() {
    let store = store_with(spend_store());
    let reply = call(
        "/global/spend/keys",
        Some(&internal_without_user()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailMessageEnvelope = decode(&reply);
    assert_eq!(body.detail.error, "No user_id found");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn keys_report_no_database_as_a_detail_error() {
    let global = call(
        "/global/spend/keys",
        Some(&admin()),
        state(None, None, Decimal::ZERO),
    )
    .await;
    assert_eq!(global.status, StatusCode::INTERNAL_SERVER_ERROR);
    let global_body: DetailMessageEnvelope = decode(&global);
    assert_eq!(global_body.detail.error, "No db connected");
    let per_user = call(
        "/global/spend/keys",
        Some(&internal("u1")),
        state(None, None, Decimal::ZERO),
    )
    .await;
    assert_eq!(per_user.status, StatusCode::INTERNAL_SERVER_ERROR);
    let per_user_body: DetailMessageEnvelope = decode(&per_user);
    assert_eq!(per_user_body.detail.error, "No db connected");
}

#[tokio::test]
async fn keys_report_no_database_before_rejecting_a_nonpositive_limit() {
    let reply = call(
        "/global/spend/keys?limit=0",
        Some(&admin()),
        state(None, None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailMessageEnvelope = decode(&reply);
    assert_eq!(body.detail.error, "No db connected");
}

#[tokio::test]
async fn keys_report_a_store_failure() {
    let mut failed = spend_store();
    failed.failure = Some(FakeFailure::InvalidAmount);
    let failing_store = store_with(failed);
    let invalid_amount_reply = call(
        "/global/spend/keys",
        Some(&admin()),
        state(Some(&failing_store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(
        invalid_amount_reply.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let invalid_amount_body: InternalEnvelope = decode(&invalid_amount_reply);
    assert_eq!(invalid_amount_body.error.message, "Internal server error");
    assert_eq!(invalid_amount_body.error.kind, "internal_server_error");
    let mut missing = spend_store();
    missing.failure = Some(FakeFailure::NoDatabase);
    let no_database_store = store_with(missing);
    let no_database_reply = call(
        "/global/spend/keys",
        Some(&admin()),
        state(Some(&no_database_store), None, Decimal::ZERO),
    )
    .await;
    let no_database_body: DetailMessageEnvelope = decode(&no_database_reply);
    assert_eq!(no_database_body.detail.error, "No db connected");
}

#[tokio::test]
async fn keys_report_an_internal_store_failure() {
    let mut failed = spend_store();
    failed.failure = Some(FakeFailure::InvalidAmount);
    let store = store_with(failed);
    let reply = call(
        "/global/spend/keys",
        Some(&internal("u1")),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: InternalEnvelope = decode(&reply);
    assert_eq!(body.error.message, "Internal server error");
    assert_eq!(body.error.kind, "internal_server_error");
}

#[tokio::test]
async fn models_default_the_limit_to_ten() {
    let mut fake = spend_store();
    fake.models = vec![ModelSpend {
        model: "gpt-4o".to_owned(),
        total_spend: decimal("2.5"),
    }];
    let store = store_with(fake);
    let reply = call(
        "/global/spend/models",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Vec<ModelBody> = decode(&reply);
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].model, "gpt-4o");
    assert_eq!(body[0].total_spend, decimal("2.5"));
    assert_eq!(store.calls(), vec!["models_by_spend(10)".to_owned()]);
}

#[rstest]
#[case::positive("/global/spend/models?limit=3", "models_by_spend(3)")]
#[case::zero("/global/spend/models?limit=0", "models_by_spend(0)")]
#[tokio::test]
async fn models_pass_a_zero_or_positive_limit_through(#[case] uri: &str, #[case] call_text: &str) {
    let store = store_with(spend_store());
    let reply = call(
        uri,
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec![call_text.to_owned()]);
}

#[tokio::test]
async fn models_reject_text_that_is_not_an_integer() {
    let store = store_with(spend_store());
    let reply = call(
        "/global/spend/models?limit=2.5",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body: ValidationEnvelope = decode(&reply);
    assert_eq!(body.detail[0].kind, "int_parsing");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn models_use_the_user_aggregation_for_an_internal_viewer() {
    let mut fake = spend_store();
    fake.user_models = vec![UserModelSpend {
        model: "gpt-4o".to_owned(),
        total_spend: decimal("2.5"),
        total_tokens: 12,
    }];
    let store = store_with(fake);
    let limited = call(
        "/global/spend/models?limit=2",
        Some(&internal("u1")),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    let limited_body: Vec<UserModelBody> = decode(&limited);
    assert_eq!(limited_body.len(), 1);
    assert_eq!(limited_body[0].model, "gpt-4o");
    assert_eq!(limited_body[0].total_spend, decimal("2.5"));
    assert_eq!(limited_body[0].total_tokens, 12);
    let default = call(
        "/global/spend/models",
        Some(&internal("u1")),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    let default_body: Vec<UserModelBody> = decode(&default);
    assert_eq!(default_body.len(), 1);
    assert_eq!(
        store.calls(),
        vec![
            "user_models_by_spend(u1,2)".to_owned(),
            "user_models_by_spend(u1,10)".to_owned()
        ]
    );
}

#[tokio::test]
async fn models_report_a_missing_internal_user_id() {
    let store = store_with(spend_store());
    let reply = call(
        "/global/spend/models",
        Some(&internal_without_user()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailMessageEnvelope = decode(&reply);
    assert_eq!(body.detail.error, "No user_id found");
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn models_report_no_database_as_a_detail_error() {
    let global = call(
        "/global/spend/models",
        Some(&admin()),
        state(None, None, Decimal::ZERO),
    )
    .await;
    assert_eq!(global.status, StatusCode::INTERNAL_SERVER_ERROR);
    let global_body: DetailMessageEnvelope = decode(&global);
    assert_eq!(global_body.detail.error, "No db connected");
    let per_user = call(
        "/global/spend/models",
        Some(&internal("u1")),
        state(None, None, Decimal::ZERO),
    )
    .await;
    assert_eq!(per_user.status, StatusCode::INTERNAL_SERVER_ERROR);
    let per_user_body: DetailMessageEnvelope = decode(&per_user);
    assert_eq!(per_user_body.detail.error, "No db connected");
}

#[tokio::test]
async fn models_report_a_store_failure() {
    let mut failed = spend_store();
    failed.failure = Some(FakeFailure::InvalidAmount);
    let store = store_with(failed);
    let reply = call(
        "/global/spend/models",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: InternalEnvelope = decode(&reply);
    assert_eq!(body.error.message, "Internal server error");
    assert_eq!(body.error.kind, "internal_server_error");
}

#[tokio::test]
async fn teams_return_the_daily_rows_and_the_ranked_totals() {
    let mut fake = spend_store();
    fake.teams = vec![
        team(Some("alpha"), "2026-01-01", Some("1.10")),
        team(Some("beta"), "2026-01-01", Some("0.25")),
        team(Some("alpha"), "2026-01-02", Some("1.00")),
        team(None, "2026-01-02", Some("0.50")),
    ];
    let store = store_with(fake);
    let reply = call(
        "/global/spend/teams",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: TeamsBody = decode(&reply);
    assert_eq!(body.daily_spend.len(), 2);
    assert_eq!(body.daily_spend[0].date, "2026-01-01");
    assert_eq!(body.daily_spend[0].amounts["alpha"], decimal("1.1"));
    assert_eq!(body.daily_spend[1].amounts["Unassigned"], decimal("0.5"));
    assert_eq!(body.teams, vec!["Unassigned", "alpha", "beta"]);
    assert_eq!(body.total_spend_per_team.len(), 3);
    assert_eq!(body.total_spend_per_team[0].team_id, "alpha");
    assert_eq!(body.total_spend_per_team[0].total_spend, decimal("2.1"));
    let text = body_text(&reply);
    assert!(text.contains(r#""alpha":1.1"#));
    assert!(!text.contains(r#""alpha":"1"#));
    assert_eq!(store.calls(), vec!["team_daily_spend".to_owned()]);
}

#[tokio::test]
async fn teams_report_no_database_as_a_detail_error() {
    let reply = call(
        "/global/spend/teams",
        Some(&admin()),
        state(None, None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailMessageEnvelope = decode(&reply);
    assert_eq!(body.detail.error, "No db connected");
}

#[tokio::test]
async fn teams_hide_a_null_total_behind_the_internal_error_shape() {
    let mut fake = spend_store();
    fake.teams = vec![team(Some("alpha"), "2026-01-01", None)];
    let store = store_with(fake);
    let reply = call(
        "/global/spend/teams",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: InternalEnvelope = decode(&reply);
    assert_eq!(body.error.message, "Internal server error");
    assert_eq!(body.error.kind, "internal_server_error");
}

#[tokio::test]
async fn teams_hide_a_store_failure_behind_the_internal_error_shape() {
    let mut fake = spend_store();
    fake.failure = Some(FakeFailure::InvalidAmount);
    let store = store_with(fake);
    let reply = call(
        "/global/spend/teams",
        Some(&admin()),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: InternalEnvelope = decode(&reply);
    assert_eq!(body.error.message, "Internal server error");
}
