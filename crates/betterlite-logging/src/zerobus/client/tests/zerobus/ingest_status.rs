use std::time::Duration;

use betterlite_logging::zerobus::client::types::ZerobusIngestFailure;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{
    FakeClock, FakeZerobus, Reply, client, client_with, closed_endpoint, connection, expected,
    token_reply, trace_rows,
};
use betterlite_models::decisions::Json;

fn minted() -> Vec<Reply> {
    vec![Reply::new(200, &token_reply("tok-1", "3600"))]
}

fn accepted() -> Vec<Reply> {
    vec![Reply::new(202, "{}")]
}

fn failing(status: u16, body: &str) -> ZerobusIngestFailure {
    ZerobusIngestFailure {
        detail: format!("insert returned {status}: {body}"),
        retryable: false,
    }
}

#[tokio::test]
async fn rows_are_posted_as_one_json_array_to_the_insert_endpoint() {
    let fake = FakeZerobus::start(minted(), accepted()).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);
    let rows = trace_rows(&[
        r#"{"id": "a", "model": "gpt-4o"}"#,
        r#"{"id": "b", "model": "gpt-4o"}"#,
    ]);

    client.insert(&rows).await.unwrap();

    let calls = fake.insert_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].headers["content-type"], "application/json");
    let body: Vec<IndexMap<String, Json>> = serde_json::from_slice(&calls[0].body).unwrap();
    assert_eq!(body.len(), 2);
    assert_eq!(body[0]["id"], expected(r#""a""#));
    assert_eq!(body[1]["id"], expected(r#""b""#));
}

#[tokio::test]
async fn an_empty_batch_is_posted_as_an_empty_array() {
    let fake = FakeZerobus::start(minted(), accepted()).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    client.insert(&[]).await.unwrap();

    assert_eq!(fake.insert_calls()[0].body_text(), "[]");
}

#[rstest]
#[case::ok(200, "{}")]
#[case::created(201, "{}")]
#[case::accepted(202, "{}")]
#[case::no_content(204, "")]
#[tokio::test]
async fn every_2xx_insert_status_is_a_success(#[case] status: u16, #[case] body: &str) {
    let fake = FakeZerobus::start(minted(), vec![Reply::new(status, body)]).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    client.insert(&[]).await.unwrap();
}

#[rstest]
#[case::request_timeout(408)]
#[case::too_many_requests(429)]
#[case::server_error(500)]
#[case::bad_gateway(502)]
#[case::service_unavailable(503)]
#[case::gateway_timeout(504)]
#[tokio::test]
async fn a_transient_insert_status_is_retryable(#[case] status: u16) {
    let fake = FakeZerobus::start(minted(), vec![Reply::new(status, "later")]).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let failure = client.insert(&[]).await.unwrap_err();

    assert!(failure.retryable);
    assert!(failure.detail.contains(&status.to_string()));
}

#[rstest]
#[case::schema_rejection(400)]
#[case::forbidden(403)]
#[case::unknown_table(404)]
#[case::conflict(409)]
#[case::not_implemented(501)]
#[tokio::test]
async fn any_other_insert_status_is_permanent(#[case] status: u16) {
    let fake = FakeZerobus::start(minted(), vec![Reply::new(status, "no")]).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let failure = client.insert(&[]).await.unwrap_err();

    assert!(!failure.retryable);
    assert_eq!(failure, failing(status, "no"));
}

#[tokio::test]
async fn a_schema_rejection_says_what_the_table_rejected() {
    let fake = FakeZerobus::start(minted(), vec![Reply::new(400, "unknown column foo")]).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let failure = client.insert(&[]).await.unwrap_err();

    assert_eq!(
        failure,
        ZerobusIngestFailure {
            detail: "insert returned 400: unknown column foo".to_owned(),
            retryable: false,
        }
    );
}

#[tokio::test]
async fn a_401_insert_discards_the_token_and_the_next_insert_mints_a_fresh_one() {
    let fake = FakeZerobus::start(
        vec![
            Reply::new(200, &token_reply("tok-1", "3600")),
            Reply::new(200, &token_reply("tok-2", "3600")),
        ],
        vec![Reply::new(401, "expired"), Reply::new(202, "{}")],
    )
    .await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let first = client.insert(&[]).await.unwrap_err();
    let second = client.insert(&[]).await;

    assert_eq!(
        first,
        ZerobusIngestFailure {
            detail: "insert returned 401, token discarded".to_owned(),
            retryable: true,
        }
    );
    second.unwrap();
    assert_eq!(fake.token_calls().len(), 2);
    let authorizations: Vec<String> = fake
        .insert_calls()
        .iter()
        .map(|call| call.headers["authorization"].clone())
        .collect();
    assert_eq!(authorizations, ["Bearer tok-1", "Bearer tok-2"]);
}

#[tokio::test]
async fn a_network_failure_on_insert_is_retryable() {
    let fake = FakeZerobus::start(minted(), accepted()).await;
    let mut connection = connection(&fake.base);
    connection.server_endpoint = closed_endpoint().await;
    let clock = FakeClock::new(1_000.0);
    let client = client_with(connection, &clock, Duration::from_secs(5));

    let failure = client.insert(&[]).await.unwrap_err();

    assert!(failure.retryable);
    assert!(failure.detail.starts_with("insert failed"));
}

#[tokio::test]
async fn a_stalled_insert_past_the_deadline_is_retryable() {
    let fake = FakeZerobus::start(minted(), vec![Reply::stalling()]).await;
    let clock = FakeClock::new(1_000.0);
    let client = client_with(connection(&fake.base), &clock, Duration::from_secs(2));

    let failure = client.insert(&[]).await.unwrap_err();

    assert!(failure.retryable);
    assert!(failure.detail.starts_with("insert failed"));
}

#[tokio::test]
async fn a_rejection_keeps_only_the_first_500_characters_of_the_body() {
    let body = "x".repeat(600);
    let fake = FakeZerobus::start(minted(), vec![Reply::new(400, &body)]).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let failure = client.insert(&[]).await.unwrap_err();

    let untruncated = format!("insert returned 400: {body}");
    let expected: String = untruncated.chars().take(500).collect();
    assert_eq!(failure.detail, expected);
    assert_eq!(failure.detail.chars().count(), 500);
}

#[tokio::test]
async fn the_rejection_body_is_truncated_by_characters_not_bytes() {
    let body = "\u{e9}".repeat(300);
    let fake = FakeZerobus::start(minted(), vec![Reply::new(400, &body)]).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let failure = client.insert(&[]).await.unwrap_err();

    assert_eq!(
        failure.detail.chars().count(),
        "insert returned 400: ".chars().count() + 300
    );
}
