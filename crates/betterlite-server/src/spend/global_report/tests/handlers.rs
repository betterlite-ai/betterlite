use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use rstest::rstest;

use crate::spend::global_report::rows::{
    CustomerSpend, DailyCustomerSpend, DailyTeamCustomerSpend, GlobalReport, TeamCustomerSpend,
};
use crate::spend::reports::rows::{ModelSpend, ScopedKeySpend, TeamModelSpend};

use super::support::{FakeStore, call, decimal, fake_store};

const DATES: &str = "start_date=2024-01-01&end_date=2024-01-31";
const WINDOW: &str = "2024-01-01T00:00:00+00:00,2024-01-31T00:00:00+00:00";
const SK_B_HASH: &str = "18519d64d0d18b0e84e43301547425933dc0394576666e8da1ef1790fb64ca9f";
const GROUP_BY_ISSUE: &str = r#"{"detail":[{"type":"literal_error","loc":["query","group_by"],"msg":"Input should be 'team', 'customer' or 'api_key'"}]}"#;
const MISSING_DATES: &str = r#"{"detail":{"error":"Please provide start_date and end_date"}}"#;
const START_DATE_FORMAT: &str = r#"{"detail":{"error":"start_date must be in YYYY-MM-DD format"}}"#;
const END_DATE_FORMAT: &str = r#"{"detail":{"error":"end_date must be in YYYY-MM-DD format"}}"#;

fn team_model() -> TeamModelSpend {
    TeamModelSpend {
        model: String::from("gpt"),
        api_key: String::from("k1"),
        spend: decimal("1.5"),
        total_tokens: 7,
    }
}

#[tokio::test]
async fn a_request_without_an_identity_answers_401() {
    let store = fake_store(GlobalReport::Keys(Vec::new()));
    let reply = call(DATES, false, Some(&store)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(store.calls().is_empty());
}

#[rstest]
#[case::empty("group_by=")]
#[case::null_text("group_by=null")]
#[case::capitalized("group_by=Team&start_date=2024-01-01&end_date=2024-01-31")]
#[case::last_value_is_invalid("group_by=customer&group_by=bogus")]
#[case::a_malformed_date("group_by=bogus&start_date=2024-13-01&end_date=2024-13-01")]
#[tokio::test]
async fn an_unknown_group_by_answers_422_before_the_dates(#[case] query: &str) {
    let store = fake_store(GlobalReport::Keys(Vec::new()));
    let reply = call(query, true, Some(&store)).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.body, GROUP_BY_ISSUE);
    assert!(store.calls().is_empty());
}

#[rstest]
#[case::no_dates("")]
#[case::start_only("start_date=2024-01-01")]
#[case::end_only("end_date=2024-01-31&api_key=sk-b")]
#[case::a_malformed_start_alone("start_date=2024-13-01")]
#[tokio::test]
async fn missing_dates_answer_400_before_the_database_check(#[case] query: &str) {
    let reply = call(query, true, None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body, MISSING_DATES);
}

#[rstest]
#[case::bad_start_month("start_date=2024-13-01&end_date=2024-01-31", START_DATE_FORMAT)]
#[case::bad_start_day("start_date=2024-01-32&end_date=2024-01-31", START_DATE_FORMAT)]
#[case::a_start_with_a_time(
    "start_date=2024-01-01T00:00:00&end_date=2024-01-31",
    START_DATE_FORMAT
)]
#[case::an_empty_start("start_date=&end_date=2024-01-31", START_DATE_FORMAT)]
#[case::bad_end_format("start_date=2024-01-01&end_date=01/31/2024", END_DATE_FORMAT)]
#[case::bad_end_day("start_date=2024-01-01&end_date=2024-02-30", END_DATE_FORMAT)]
#[case::an_empty_end("start_date=2024-01-01&end_date=", END_DATE_FORMAT)]
#[case::both_malformed_reads_the_start_first(
    "start_date=2024-13-01&end_date=2024-13-01",
    START_DATE_FORMAT
)]
#[tokio::test]
async fn a_malformed_date_answers_400_before_the_database_check(
    #[case] query: &str,
    #[case] expected: &str,
) {
    let reply = call(query, true, None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body, expected);
}

#[tokio::test]
async fn a_missing_database_answers_400_with_detail_error() {
    let reply = call(DATES, true, None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body,
        r#"{"detail":{"error":"DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys"}}"#
    );
}

#[tokio::test]
async fn a_failing_query_answers_400_with_detail_error() {
    let store = Arc::new(FakeStore {
        reply: GlobalReport::Keys(Vec::new()),
        failure: true,
        calls: Mutex::new(Vec::new()),
    });
    let reply = call(DATES, true, Some(&store)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.body,
        r#"{"detail":{"error":"No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"}}"#
    );
}

#[rstest]
#[case::every_filter_picks_the_hashed_key(
    "api_key=sk-b&internal_user_id=u1&team_id=t1&customer_id=c1&group_by=customer",
    "scoped(ApiKey,{SK_B_HASH})"
)]
#[case::the_last_repeated_key_wins("api_key=sk-a&api_key=sk-b", "scoped(ApiKey,{SK_B_HASH})")]
#[case::a_hash_is_used_as_given("api_key=sk_hash&group_by=api_key", "scoped(ApiKey,sk_hash)")]
#[case::an_empty_key_still_picks_the_key("api_key=&internal_user_id=u1", "scoped(ApiKey,)")]
#[case::user_before_team_customer(
    "internal_user_id=u1&team_id=t1&customer_id=c1",
    "scoped(User,u1)"
)]
#[case::an_empty_user_still_picks_the_user("internal_user_id=&team_id=t1", "scoped(User,)")]
#[case::team_with_customer_before_group_by(
    "team_id=t1&customer_id=c1&group_by=api_key",
    "team_customer(t1,c1)"
)]
#[case::empty_team_and_customer("team_id=&customer_id=", "team_customer(,)")]
#[case::team_alone_filters_the_team_group("team_id=t1", r#"teams(Some("t1"))"#)]
#[case::the_team_group_is_the_default("", "teams(None)")]
#[case::customer_alone_is_ignored_by_teams("customer_id=c1&group_by=team", "teams(None)")]
#[case::customer_alone_is_ignored_by_customers("group_by=customer&customer_id=c1", "customers")]
#[case::team_alone_is_ignored_by_customers("group_by=customer&team_id=t1", "customers")]
#[case::team_alone_is_ignored_by_keys("group_by=api_key&team_id=t1", "keys")]
#[case::the_last_group_by_wins("group_by=api_key&group_by=customer", "customers")]
#[tokio::test]
async fn filters_pick_the_legacy_branch(#[case] filters: &str, #[case] expected: &str) {
    let store = fake_store(GlobalReport::Keys(Vec::new()));
    let reply = call(&format!("{DATES}&{filters}"), true, Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, "[]");
    let branch = expected.replace("{SK_B_HASH}", SK_B_HASH);
    assert_eq!(store.calls(), vec![format!("{branch}@{WINDOW}")]);
}

#[rstest]
#[case::reversed_dates(
    "start_date=2024-02-01&end_date=2024-01-01",
    "teams(None)@2024-02-01T00:00:00+00:00,2024-01-01T00:00:00+00:00"
)]
#[case::longer_than_a_year(
    "start_date=2020-01-01&end_date=2024-12-31",
    "teams(None)@2020-01-01T00:00:00+00:00,2024-12-31T00:00:00+00:00"
)]
#[case::single_digit_month_and_day(
    "start_date=2024-1-5&end_date=2024-1-31",
    "teams(None)@2024-01-05T00:00:00+00:00,2024-01-31T00:00:00+00:00"
)]
#[tokio::test]
async fn the_dates_bind_as_utc_midnights_without_range_checks(
    #[case] query: &str,
    #[case] expected: &str,
) {
    let store = fake_store(GlobalReport::Keys(Vec::new()));
    let reply = call(query, true, Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec![String::from(expected)]);
}

#[tokio::test]
async fn an_authorized_caller_reads_the_report_without_a_license() {
    let store = fake_store(GlobalReport::Keys(vec![ScopedKeySpend {
        api_key: String::from("k1"),
        total_cost: decimal("3.5"),
        total_input_tokens: 1,
        total_output_tokens: 2,
        model_details: vec![ModelSpend {
            model: String::from("gpt"),
            total_cost: decimal("3.5"),
            total_input_tokens: 1,
            total_output_tokens: 2,
        }],
    }]));
    let reply = call(&format!("{DATES}&group_by=api_key"), true, Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        r#"[{"api_key":"k1","total_cost":3.5,"total_input_tokens":1,"total_output_tokens":2,"model_details":[{"model":"gpt","total_cost":3.5,"total_input_tokens":1,"total_output_tokens":2}]}]"#
    );
}

#[tokio::test]
async fn team_customer_rows_render_as_teams_customers() {
    let store = fake_store(GlobalReport::TeamsCustomers(vec![DailyTeamCustomerSpend {
        group_by_day: String::from("2024-01-01T00:00:00.000Z"),
        teams_customers: vec![TeamCustomerSpend {
            team_name: String::from("Unassigned Team"),
            customer: String::from("c1"),
            total_spend: decimal("1.5"),
            metadata: vec![team_model()],
        }],
    }]));
    let reply = call(
        &format!("{DATES}&team_id=t1&customer_id=c1"),
        true,
        Some(&store),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        r#"[{"group_by_day":"2024-01-01T00:00:00.000Z","teams_customers":[{"team_name":"Unassigned Team","customer":"c1","total_spend":1.5,"metadata":[{"model":"gpt","api_key":"k1","spend":1.5,"total_tokens":7}]}]}]"#
    );
}

#[tokio::test]
async fn customer_rows_render_a_missing_end_user_as_null() {
    let store = fake_store(GlobalReport::Customers(vec![DailyCustomerSpend {
        group_by_day: String::from("2024-01-01T00:00:00.000Z"),
        customers: vec![CustomerSpend {
            customer: None,
            total_spend: decimal("1.5"),
            metadata: vec![team_model()],
        }],
    }]));
    let reply = call(&format!("{DATES}&group_by=customer"), true, Some(&store)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        r#"[{"group_by_day":"2024-01-01T00:00:00.000Z","customers":[{"customer":null,"total_spend":1.5,"metadata":[{"model":"gpt","api_key":"k1","spend":1.5,"total_tokens":7}]}]}]"#
    );
}
