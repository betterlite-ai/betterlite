use std::time::Duration;

use super::super::support::{FakeClock, FakeZerobus, Reply, client_with, connection, token_reply};

#[tokio::test]
async fn the_token_url_comes_from_the_workspace_url_without_its_trailing_slashes() {
    let fake = FakeZerobus::start(
        vec![Reply::new(200, &token_reply("tok-1", "3600"))],
        vec![Reply::new(202, "{}")],
    )
    .await;
    let mut connection = connection(&fake.base);
    connection.workspace_url = format!("{}/", fake.base);
    let clock = FakeClock::new(1_000.0);
    let client = client_with(connection, &clock, Duration::from_secs(5));

    client.insert(&[]).await.unwrap();

    let calls = fake.token_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(calls[0].path, "/oidc/v1/token");
}

#[tokio::test]
async fn an_endpoint_that_is_not_a_url_is_a_permanent_failure() {
    let fake = FakeZerobus::start(
        vec![Reply::new(200, &token_reply("tok-1", "3600"))],
        vec![Reply::new(202, "{}")],
    )
    .await;
    let mut connection = connection(&fake.base);
    connection.workspace_url = "not a url".to_owned();
    let clock = FakeClock::new(1_000.0);
    let client = client_with(connection, &clock, Duration::from_secs(5));

    let failure = client.insert(&[]).await.unwrap_err();

    assert!(!failure.retryable);
    assert!(failure.detail.contains("token URL"));
}

#[tokio::test]
async fn the_insert_url_comes_from_the_server_endpoint_and_the_table_name() {
    let fake = FakeZerobus::start(
        vec![Reply::new(200, &token_reply("tok-1", "3600"))],
        vec![Reply::new(202, "{}")],
    )
    .await;
    let mut connection = connection(&fake.base);
    connection.server_endpoint = format!("{}///", fake.base);
    let clock = FakeClock::new(1_000.0);
    let client = client_with(connection, &clock, Duration::from_secs(5));

    client.insert(&[]).await.unwrap();

    let calls = fake.insert_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(
        calls[0].path,
        "/zerobus/v1/tables/main.litellm.traces/insert"
    );
}
