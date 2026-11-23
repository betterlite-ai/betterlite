use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::table_ops::rows::SpendLogRow;
use betterlite_models::key_digest::hashing::hash_token;
use rstest::rstest;

use super::support::{
    Events, Failure, Fake, Reply, admin, call, internal, internal_without_user, json, row, seen,
    state, summary_row, viewer,
};
use crate::spend::log_results::capped::TRUNCATION_HEADER;

fn first_row(reply: &Reply) -> BTreeMap<String, JsonData> {
    let JsonData::Array(rows) = reply.json() else {
        panic!("the body is not an array");
    };
    let Some(JsonData::Object(fields)) = rows.into_iter().next() else {
        panic!("the first row is not an object");
    };
    fields
}

fn many_rows(count: usize) -> Vec<SpendLogRow> {
    (0..count)
        .map(|index| row(&format!("req-{index}"), "2024-01-01T00:00:00+00:00"))
        .collect()
}

const NO_DATABASE_BODY: &str = r#"{"error":{"message":"/spend/logs ErrorDatabase not connected. Connect a database to your proxy - https://docs.litellm.ai/docs/simple_proxy#managing-auth---virtual-keys","type":"internal_error","param":"None","code":"500"}}"#;
const STORE_DATABASE_BODY: &str = r#"{"error":{"message":"/spend/logs ErrorNo DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys","type":"internal_error","param":"None","code":"500"}}"#;
const BAD_DATE_BODY: &str = r#"{"error":{"message":"/spend/logs Errortime data '2024-13-01' does not match format '%Y-%m-%d'","type":"internal_error","param":"None","code":"500"}}"#;

#[tokio::test]
async fn an_internal_user_reads_only_their_own_rows() {
    let fake = Fake {
        events: Events::default(),
        rows: vec![row("req-1", "2024-01-02T00:00:00+00:00")],
        summary: Vec::new(),
        aggregates: Vec::new(),
        row_failure: None,
        summary_failure: None,
        aggregate_failure: None,
    };
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake))),
        "/spend/logs?user_id=root&api_key=sk-live&request_id=r1&start_date=2024-01-02&end_date=2024-01-03&summarize=false",
        Some(&internal("alice")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let key = hash_token("sk-live");
    let expected =
        format!("rows 2024-01-02T00:00:00+00:00|2024-01-03T00:00:00+00:00|{key}|r1|alice ");
    let recorded = seen(&events);
    assert_eq!(recorded.len(), 1);
    let event = &recorded[0];
    assert!(event.starts_with(&expected));
    assert!(event.contains("(t.request_id = $4 OR t.litellm_call_id = $4)"));
    assert!(event.ends_with("ORDER BY t.\"startTime\" DESC LIMIT 10000"));
    let fields = first_row(&reply);
    assert_eq!(fields.get("request_id"), Some(&json("\"req-1\"")));
    assert_eq!(
        fields.get("startTime"),
        Some(&json("\"2024-01-02T00:00:00Z\""))
    );
}

// LiteLLM assigns user_id = user_api_key_dict.user_id even when it is None, clearing a filter
#[tokio::test]
async fn an_internal_user_without_a_user_id_reads_every_user() {
    let fake = Fake::default();
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake))),
        "/spend/logs?user_id=root&summarize=false",
        Some(&internal_without_user()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = seen(&events);
    assert_eq!(recorded.len(), 1);
    let event = &recorded[0];
    assert!(event.starts_with("rows  "));
    assert!(!event.contains("\"user\""));
}

#[tokio::test]
async fn an_admin_keeps_the_requested_user() {
    let fake = Fake::default();
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake))),
        "/spend/logs?user_id=bob&summarize=false",
        Some(&admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = seen(&events);
    assert_eq!(recorded.len(), 1);
    let event = &recorded[0];
    assert!(event.starts_with("rows bob "));
    assert!(!event.contains("root"));
}

#[tokio::test]
async fn a_viewer_keeps_the_requested_user() {
    let fake = Fake::default();
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake))),
        "/spend/logs?user_id=bob&summarize=false",
        Some(&viewer()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = seen(&events);
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].starts_with("rows bob "));
}

#[tokio::test]
async fn an_api_key_that_is_not_sk_is_read_as_is() {
    let fake = Fake::default();
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake))),
        "/spend/logs?api_key=raw-key&summarize=false",
        Some(&admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = seen(&events);
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].starts_with("rows raw-key "));
}

// one date is never parsed, so a malformed one cannot fail the request
#[rstest]
#[case::start_only("/spend/logs?start_date=2024-01-02")]
#[case::end_only("/spend/logs?end_date=2024-01-03&summarize=true")]
#[case::invalid_start("/spend/logs?start_date=2024-13-01")]
#[tokio::test]
async fn one_date_alone_lists_rows_and_ignores_summarize(#[case] uri: &str) {
    let fake = Fake::default();
    let events = Arc::clone(&fake.events);
    let reply = call(state(Some(Arc::new(fake))), uri, Some(&admin())).await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = seen(&events);
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].starts_with("rows  "));
    assert!(!recorded[0].contains(">="));
}

#[tokio::test]
async fn a_summary_pads_only_after_the_last_day_through_end_date() {
    let fake = Fake {
        events: Events::default(),
        rows: Vec::new(),
        summary: vec![summary_row(
            "2024-01-02",
            Some("k1"),
            Some("alice"),
            "gpt",
            "1.5",
        )],
        aggregates: Vec::new(),
        row_failure: None,
        summary_failure: None,
        aggregate_failure: None,
    };
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake))),
        "/spend/logs?api_key=sk-live&request_id=r1&start_date=2024-01-02&end_date=2024-01-04",
        Some(&internal("alice")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json(),
        json(
            r#"[{"k1":1.5,"startTime":"2024-01-02","spend":1.5,"users":{"alice":1.5},"models":{"gpt":1.5}},{"startTime":"2024-01-03","spend":0,"users":{},"models":{}},{"startTime":"2024-01-04","spend":0,"users":{},"models":{}}]"#
        )
    );
    let key = hash_token("sk-live");
    let expected =
        format!("summary 2024-01-02T00:00:00+00:00|2024-01-04T00:00:00+00:00|{key}|r1|alice ");
    let recorded = seen(&events);
    assert_eq!(recorded.len(), 1);
    let event = &recorded[0];
    assert!(event.starts_with(&expected));
    assert!(event.contains("AND request_id = $4"));
    assert!(!event.contains("litellm_call_id"));
}

#[tokio::test]
async fn an_empty_summary_result_is_not_padded() {
    let fake = Fake::default();
    let reply = call(
        state(Some(Arc::new(fake))),
        "/spend/logs?start_date=2024-01-02&end_date=2024-01-04",
        Some(&internal("alice")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json("[]"));
}

#[tokio::test]
async fn no_database_answers_the_exact_error() {
    let reply = call(state(None), "/spend/logs", Some(&internal("alice"))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.json(), json(NO_DATABASE_BODY));
}

// the leniency chrono adds over strptime, listed in docs/deviations/B1730.yaml
#[tokio::test]
async fn a_signed_year_is_forwarded_instead_of_refused() {
    let fake = Fake::default();
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake))),
        "/spend/logs?start_date=-100-01-01&end_date=2024-01-01&summarize=false",
        Some(&admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let recorded = seen(&events);
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].starts_with("rows -0100-01-01T00:00:00+00:00|2024-01-01T00:00:00+00:00 "));
}

#[tokio::test]
async fn no_database_wins_over_a_bad_date() {
    let reply = call(
        state(None),
        "/spend/logs?start_date=2024-13-01&end_date=2024-01-02",
        Some(&internal("alice")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.json(), json(NO_DATABASE_BODY));
}

#[tokio::test]
async fn a_bad_date_answers_the_parse_error() {
    let fake = Fake::default();
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake))),
        "/spend/logs?start_date=2024-13-01&end_date=2024-01-02",
        Some(&admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.json(), json(BAD_DATE_BODY));
    assert!(seen(&events).is_empty());
}

#[tokio::test]
async fn exactly_ten_thousand_rows_set_the_truncation_header() {
    let fake = Fake {
        events: Events::default(),
        rows: many_rows(10_000),
        summary: Vec::new(),
        aggregates: Vec::new(),
        row_failure: None,
        summary_failure: None,
        aggregate_failure: None,
    };
    let reply = call(state(Some(Arc::new(fake))), "/spend/logs", Some(&admin())).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.header(TRUNCATION_HEADER), Some(String::from("true")));
}

#[tokio::test]
async fn nine_thousand_nine_hundred_ninety_nine_rows_do_not() {
    let fake = Fake {
        events: Events::default(),
        rows: many_rows(9_999),
        summary: Vec::new(),
        aggregates: Vec::new(),
        row_failure: None,
        summary_failure: None,
        aggregate_failure: None,
    };
    let reply = call(state(Some(Arc::new(fake))), "/spend/logs", Some(&admin())).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.header(TRUNCATION_HEADER), None);
}

#[rstest]
#[case::database(
    Failure::Database,
    StatusCode::INTERNAL_SERVER_ERROR,
    STORE_DATABASE_BODY
)]
#[case::row(
    Failure::Row,
    StatusCode::INTERNAL_SERVER_ERROR,
    r#"{"error":{"message":"/spend/logs Errorthe stored row is not a JSON object","type":"internal_error","param":"None","code":"500"}}"#
)]
#[case::http(
    Failure::Http,
    StatusCode::FORBIDDEN,
    r#"{"error":{"message":"no team budget","type":"internal_error","param":"None","code":"403"}}"#
)]
#[case::proxy(
    Failure::Proxy,
    StatusCode::TOO_MANY_REQUESTS,
    r#"{"error":{"message":"budget exceeded","type":"budget_exceeded","param":null,"code":"429"}}"#
)]
#[tokio::test]
async fn a_store_failure_answers_with_litellm_status_and_body(
    #[case] failure: Failure,
    #[case] status: StatusCode,
    #[case] body: &str,
) {
    let fake = Fake {
        events: Events::default(),
        rows: Vec::new(),
        summary: Vec::new(),
        aggregates: Vec::new(),
        row_failure: Some(failure),
        summary_failure: None,
        aggregate_failure: None,
    };
    let reply = call(state(Some(Arc::new(fake))), "/spend/logs", Some(&admin())).await;
    assert_eq!(reply.status, status);
    assert_eq!(reply.json(), json(body));
}

#[tokio::test]
async fn a_summary_failure_wraps_into_the_internal_error() {
    let fake = Fake {
        events: Events::default(),
        rows: Vec::new(),
        summary: Vec::new(),
        aggregates: Vec::new(),
        row_failure: None,
        summary_failure: Some(Failure::Database),
        aggregate_failure: None,
    };
    let reply = call(
        state(Some(Arc::new(fake))),
        "/spend/logs?start_date=2024-01-02&end_date=2024-01-03",
        Some(&admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.json(), json(STORE_DATABASE_BODY));
}

#[tokio::test]
async fn an_unparsable_summarize_answers_the_framework_422() {
    let reply = call(
        state(Some(Arc::new(Fake::default()))),
        "/spend/logs?summarize=maybe",
        Some(&admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.json(),
        json(
            r#"{"detail":[{"type":"bool_parsing","loc":["query","summarize"],"msg":"Input should be a valid boolean, unable to interpret input"}]}"#
        )
    );
    // the query is validated before the endpoint checks for a database
    let without_database = call(state(None), "/spend/logs?summarize=maybe", Some(&admin())).await;
    assert_eq!(without_database.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(without_database.json(), reply.json());
}

#[tokio::test]
async fn no_identity_answers_unauthorized() {
    let reply = call(state(Some(Arc::new(Fake::default()))), "/spend/logs", None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        reply.json(),
        json(
            r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#
        )
    );
}
