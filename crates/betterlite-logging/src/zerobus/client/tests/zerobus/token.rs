use std::time::Duration;

use betterlite_logging::zerobus::client::types::{ZerobusAccessToken, ZerobusIngestFailure};
use futures_util::future::join_all;
use secrecy::{ExposeSecret as _, SecretString};

use super::support::{
    FakeClock, FakeZerobus, Reply, client, client_with, closed_endpoint, connection, token_reply,
};

fn accepted() -> Vec<Reply> {
    vec![Reply::new(202, "{}")]
}

#[tokio::test]
async fn a_token_is_reused_while_more_than_the_leeway_is_left() {
    let fake = FakeZerobus::start(
        vec![
            Reply::new(200, &token_reply("tok-1", "600")),
            Reply::new(200, &token_reply("tok-2", "600")),
        ],
        accepted(),
    )
    .await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    client.insert(&[]).await.unwrap();
    clock.set(1_539.0);
    client.insert(&[]).await.unwrap();
    clock.set(1_540.0);
    client.insert(&[]).await.unwrap();

    assert_eq!(fake.token_calls().len(), 2);
    let authorizations: Vec<String> = fake
        .insert_calls()
        .iter()
        .map(|call| call.headers["authorization"].clone())
        .collect();
    assert_eq!(
        authorizations,
        ["Bearer tok-1", "Bearer tok-1", "Bearer tok-2"]
    );
}

#[tokio::test]
async fn a_token_response_without_expires_in_is_cached_for_an_hour() {
    let fake = FakeZerobus::start(
        vec![
            Reply::new(200, r#"{"access_token": "tok-1"}"#),
            Reply::new(200, r#"{"access_token": "tok-2"}"#),
        ],
        accepted(),
    )
    .await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    client.insert(&[]).await.unwrap();
    clock.set(4_539.0);
    client.insert(&[]).await.unwrap();
    clock.set(4_540.0);
    client.insert(&[]).await.unwrap();

    assert_eq!(fake.token_calls().len(), 2);
}

#[tokio::test]
async fn the_access_token_is_the_minted_token_for_the_table() {
    let fake = FakeZerobus::start(
        vec![Reply::new(200, &token_reply("tok-1", "3600"))],
        accepted(),
    )
    .await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let token = client.access_token().await.unwrap();

    assert_eq!(token.value.expose_secret(), "tok-1");
}

#[tokio::test]
async fn a_failed_mint_leaves_the_previous_token_out_of_the_cache() {
    let fake = FakeZerobus::start(
        vec![
            Reply::new(200, &token_reply("tok-1", "600")),
            Reply::new(503, "try later"),
            Reply::new(200, &token_reply("tok-2", "600")),
        ],
        accepted(),
    )
    .await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    client.insert(&[]).await.unwrap();
    clock.set(2_000.0);
    let failure = client.insert(&[]).await.unwrap_err();
    assert!(failure.retryable);
    client.insert(&[]).await.unwrap();

    assert_eq!(fake.token_calls().len(), 3);
    let authorizations: Vec<String> = fake
        .insert_calls()
        .iter()
        .map(|call| call.headers["authorization"].clone())
        .collect();
    assert_eq!(authorizations, ["Bearer tok-1", "Bearer tok-2"]);
}

#[tokio::test]
async fn a_401_on_the_token_request_is_permanent_and_no_rows_are_posted() {
    let fake = FakeZerobus::start(vec![Reply::new(401, "invalid_client")], accepted()).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let failure = client.insert(&[]).await.unwrap_err();

    assert_eq!(
        failure,
        ZerobusIngestFailure {
            detail: "token request returned 401: invalid_client".to_owned(),
            retryable: false,
        }
    );
    assert!(fake.insert_calls().is_empty());
}

#[tokio::test]
async fn a_token_endpoint_outage_is_retryable() {
    let fake = FakeZerobus::start(vec![Reply::new(503, "try later")], accepted()).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let failure = client.insert(&[]).await.unwrap_err();

    assert!(failure.retryable);
    assert!(failure.detail.starts_with("token request returned 503"));
    assert!(fake.insert_calls().is_empty());
}

#[tokio::test]
async fn a_token_response_without_a_token_is_a_permanent_failure() {
    let fake = FakeZerobus::start(
        vec![Reply::new(200, r#"{"token_type": "Bearer"}"#)],
        accepted(),
    )
    .await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let failure = client.insert(&[]).await.unwrap_err();

    assert!(!failure.retryable);
    assert!(failure.detail.contains("token response"));
}

#[tokio::test]
async fn a_malformed_token_response_is_a_permanent_failure() {
    let fake = FakeZerobus::start(vec![Reply::new(200, "not json")], accepted()).await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let failure = client.insert(&[]).await.unwrap_err();

    assert!(!failure.retryable);
    assert!(failure.detail.contains("token response"));
}

#[tokio::test]
async fn a_transport_failure_on_the_token_request_is_retryable() {
    let fake = FakeZerobus::start(
        vec![Reply::new(200, &token_reply("tok-1", "3600"))],
        accepted(),
    )
    .await;
    let mut connection = connection(&fake.base);
    connection.workspace_url = closed_endpoint().await;
    let clock = FakeClock::new(1_000.0);
    let client = client_with(connection, &clock, Duration::from_secs(5));

    let failure = client.insert(&[]).await.unwrap_err();

    assert!(failure.retryable);
    assert!(failure.detail.starts_with("token request failed"));
    assert!(fake.insert_calls().is_empty());
}

#[tokio::test]
async fn concurrent_inserts_share_one_token_request() {
    let fake = FakeZerobus::start(
        vec![Reply::new(200, &token_reply("tok-1", "3600"))],
        accepted(),
    )
    .await;
    let clock = FakeClock::new(1_000.0);
    let client = client(&fake, &clock);

    let outcomes = join_all([client.insert(&[]), client.insert(&[]), client.insert(&[])]).await;

    assert!(outcomes.iter().all(Result::is_ok));
    assert_eq!(fake.token_calls().len(), 1);
    assert!(
        fake.insert_calls()
            .iter()
            .all(|call| call.headers["authorization"] == "Bearer tok-1")
    );
}

#[test]
fn the_minted_token_stays_out_of_debug_output() {
    let token = ZerobusAccessToken {
        value: SecretString::from("tok-secret"),
        expires_at: 1.0,
    };
    assert!(!format!("{token:?}").contains("tok-secret"));
}
