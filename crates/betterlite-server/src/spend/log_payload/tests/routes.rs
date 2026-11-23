use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use rstest::rstest;

use super::support::{Answer, FakeKind, Fixture, admin, internal, json, seen, stored_row, viewer};

const OWN_PAYLOAD: &str = r#"{"messages":"own","metadata":{"user_api_key_user_id":"alice"}}"#;
const FOREIGN_PAYLOAD: &str =
    r#"{"messages":"foreign","metadata":{"user_api_key_user_id":"mallory"}}"#;
const NO_DATABASE: &str = "Cannot authorize spend log access without a database connection. Connect a database or use a proxy admin key.";

#[tokio::test]
async fn refuses_a_request_without_an_identity() {
    let reply = Fixture::new(Some(Vec::new()))
        .call("/spend/logs/ui/req-1", None)
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_non_admin_without_a_database_is_refused_before_any_callback() {
    let mut fixture = Fixture::new(None);
    fixture.logger(
        FakeKind::Payload,
        "first",
        Answer::Found(OWN_PAYLOAD.to_owned()),
    );
    let events = Arc::clone(&fixture.events);
    let reply = fixture
        .call(
            "/spend/logs/ui/req-1?start_date=bad",
            Some(&internal("alice")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.json(),
        json(&format!(r#"{{"detail":{{"error":"{NO_DATABASE}"}}}}"#))
    );
    assert!(seen(&events).is_empty());
}

#[tokio::test]
async fn a_foreign_id_is_refused_before_its_dates_are_read() {
    let mut fixture = Fixture::new(Some(vec![stored_row("req-1", None, Some("mallory"))]));
    fixture.logger(
        FakeKind::Payload,
        "first",
        Answer::Found(OWN_PAYLOAD.to_owned()),
    );
    let events = Arc::clone(&fixture.events);
    let reply = fixture
        .call(
            "/spend/logs/ui/req-1?start_date=not-a-date",
            Some(&internal("alice")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.json(),
        json(r#"{"detail":{"error":"Not authorized to view spend log for request_id=req-1"}}"#)
    );
    assert_eq!(seen(&events), ["owners req-1"]);
}

#[rstest]
#[case::owner_with_bad_start(internal("alice"), "start_date=2025-01-01")]
#[case::owner_with_bad_end(
    internal("alice"),
    "start_date=2025-01-01%2000:00:00&end_date=2025-01-02T00:00:00"
)]
#[case::admin_with_bad_start(admin(), "start_date=")]
#[tokio::test]
async fn an_authorized_bad_date_is_a_server_error(#[case] caller: Principal, #[case] query: &str) {
    let mut fixture = Fixture::new(Some(vec![stored_row("req-1", None, Some("alice"))]));
    fixture.logger(
        FakeKind::Payload,
        "first",
        Answer::Found(OWN_PAYLOAD.to_owned()),
    );
    let events = Arc::clone(&fixture.events);
    let reply = fixture
        .call(&format!("/spend/logs/ui/req-1?{query}"), Some(&caller))
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        seen(&events)
            .iter()
            .all(|event| !event.starts_with("callback"))
    );
}

#[tokio::test]
async fn asks_callbacks_in_order_and_serves_the_first_payload() {
    let mut fixture = Fixture::new(Some(vec![stored_row("req-1", None, Some("alice"))]));
    fixture.logger(FakeKind::Payload, "first", Answer::Missing);
    fixture.logger(
        FakeKind::Payload,
        "second",
        Answer::Found(OWN_PAYLOAD.to_owned()),
    );
    fixture.logger(
        FakeKind::Payload,
        "third",
        Answer::Found(FOREIGN_PAYLOAD.to_owned()),
    );
    let events = Arc::clone(&fixture.events);
    let reply = fixture
        .call(
            "/spend/logs/ui/req-1?start_date=2025-01-02%2003:04:05&end_date=2025-01-03+00:00:00",
            Some(&internal("alice")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json(OWN_PAYLOAD));
    let window = r#"Some("2025-01-02T03:04:05+00:00") Some("2025-01-03T00:00:00+00:00")"#;
    assert_eq!(
        seen(&events),
        [
            "owners req-1".to_owned(),
            "payload_rows req-1".to_owned(),
            format!("callback first req-1 {window}"),
            format!("callback second req-1 {window}"),
        ]
    );
}

#[tokio::test]
async fn a_failing_callback_is_a_server_error() {
    let mut fixture = Fixture::new(Some(vec![stored_row("req-1", None, Some("alice"))]));
    fixture.logger(FakeKind::Payload, "first", Answer::Fails);
    fixture.logger(
        FakeKind::Payload,
        "second",
        Answer::Found(OWN_PAYLOAD.to_owned()),
    );
    let reply = fixture
        .call("/spend/logs/ui/req-1", Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn a_foreign_payload_under_a_shared_call_id_is_refused() {
    let rows = vec![
        stored_row("req-own", Some("call-shared"), Some("alice")),
        stored_row("req-foreign", Some("call-shared"), Some("mallory")),
    ];
    let mut fixture = Fixture::new(Some(rows));
    fixture.logger(
        FakeKind::Payload,
        "first",
        Answer::Found(FOREIGN_PAYLOAD.to_owned()),
    );
    let events = Arc::clone(&fixture.events);
    let reply = fixture
        .call("/spend/logs/ui/call-shared", Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.json(),
        json(
            r#"{"detail":{"error":"Not authorized to view spend log for request_id=call-shared"}}"#
        )
    );
    assert!(seen(&events).contains(&"callback first req-own None None".to_owned()));
}

#[rstest]
#[case::admin(admin())]
#[case::viewer(viewer())]
#[tokio::test]
async fn an_admin_is_served_any_callback_payload(#[case] caller: Principal) {
    let mut fixture = Fixture::new(Some(vec![stored_row("req-1", None, Some("mallory"))]));
    fixture.logger(
        FakeKind::Payload,
        "first",
        Answer::Found(FOREIGN_PAYLOAD.to_owned()),
    );
    let events = Arc::clone(&fixture.events);
    let reply = fixture.call("/spend/logs/ui/req-1", Some(&caller)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json(FOREIGN_PAYLOAD));
    assert_eq!(
        seen(&events),
        ["payload_rows req-1", "callback first req-1 None None"]
    );
}

#[rstest]
#[case::payload(Answer::Found(FOREIGN_PAYLOAD.to_owned()), FOREIGN_PAYLOAD)]
#[case::nothing(Answer::Missing, "null")]
#[tokio::test]
async fn an_admin_without_a_database_reads_callbacks_by_the_url_id(
    #[case] answer: Answer,
    #[case] expected: &str,
) {
    let mut fixture = Fixture::new(None);
    fixture.logger(FakeKind::Payload, "first", answer);
    let events = Arc::clone(&fixture.events);
    let reply = fixture
        .call(
            "/spend/logs/ui/call-7?end_date=2025-05-06%2007:08:09",
            Some(&admin()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json(expected));
    assert_eq!(
        seen(&events),
        [r#"callback first call-7 None Some("2025-05-06T07:08:09+00:00")"#]
    );
}

#[tokio::test]
async fn the_exact_request_id_outranks_a_colliding_call_id() {
    let mut own = stored_row("req-a", Some("req-b"), Some("alice"));
    own.row.messages = Some(json(r#""by call id""#));
    let mut exact = stored_row("req-b", None, Some("alice"));
    exact.row.messages = Some(json(r#""by request id""#));
    let mut fixture = Fixture::new(Some(vec![own, exact]));
    fixture.logger(FakeKind::Payload, "first", Answer::Missing);
    let events = Arc::clone(&fixture.events);
    let reply = fixture
        .call("/spend/logs/ui/req-b", Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json(),
        json(
            r#"{"messages":"by request id","response":{"id":"resp"},"proxy_server_request":{"model":"m"}}"#
        )
    );
    assert!(seen(&events).contains(&"callback first req-b None None".to_owned()));
}

#[tokio::test]
async fn a_call_id_lookup_asks_callbacks_for_the_stored_request_id() {
    let mut fixture = Fixture::new(Some(vec![stored_row(
        "req-9",
        Some("call-9"),
        Some("alice"),
    )]));
    fixture.logger(
        FakeKind::Payload,
        "first",
        Answer::Found(OWN_PAYLOAD.to_owned()),
    );
    let events = Arc::clone(&fixture.events);
    let reply = fixture
        .call("/spend/logs/ui/call-9", Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json(OWN_PAYLOAD));
    assert!(seen(&events).contains(&"callback first req-9 None None".to_owned()));
}

#[tokio::test]
async fn no_payload_and_no_row_answers_null() {
    let mut fixture = Fixture::new(Some(Vec::new()));
    fixture.logger(FakeKind::Payload, "first", Answer::Missing);
    let reply = fixture
        .call("/spend/logs/ui/req-gone", Some(&admin()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
}

#[tokio::test]
async fn a_row_without_a_callback_payload_is_resolved_through_cold_storage() {
    let mut placeholder = stored_row("req-1", None, Some("alice"));
    placeholder.row.messages = Some(json(r#""{}""#));
    placeholder.row.response = Some(json(r#""{}""#));
    placeholder.row.proxy_server_request = Some(json(r#""{}""#));
    placeholder.row.metadata = Some(json(r#""{\"cold_storage_object_key\":\"k-1\"}""#));
    let mut fixture = Fixture::new(Some(vec![placeholder]));
    fixture.logger(
        FakeKind::Cold,
        "s3_v2",
        Answer::Found(r#"{"messages":["cold"],"response":"cold"}"#.to_owned()),
    );
    let events = Arc::clone(&fixture.events);
    let reply = fixture
        .call("/spend/logs/ui/req-1", Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json(),
        json(r#"{"messages":["cold"],"response":"cold","proxy_server_request":null}"#)
    );
    assert_eq!(
        seen(&events).last().map(String::as_str),
        Some("cold s3_v2 k-1")
    );
}
