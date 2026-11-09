use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::super::support::{
    CLIENT_ID, CLIENT_SECRET, FakeClock, FakeZerobus, Reply, client_with, connection, token_reply,
};

async fn fake() -> FakeZerobus {
    FakeZerobus::start(
        vec![Reply::new(200, &token_reply("tok-1", "3600"))],
        vec![Reply::new(202, "{}")],
    )
    .await
}

#[tokio::test]
async fn the_token_request_authenticates_the_service_principal_with_http_basic() {
    let fake = fake().await;
    let clock = FakeClock::new(1_000.0);
    let client = client_with(connection(&fake.base), &clock, Duration::from_secs(5));

    client.insert(&[]).await.unwrap();

    let calls = fake.token_calls();
    let authorization = &calls[0].headers["authorization"];
    let (scheme, encoded) = authorization.split_once(' ').unwrap();
    assert_eq!(scheme, "Basic");
    let decoded = STANDARD.decode(encoded).unwrap();
    assert_eq!(
        String::from_utf8(decoded).unwrap(),
        format!("{CLIENT_ID}:{CLIENT_SECRET}")
    );
}

#[tokio::test]
async fn the_insert_request_carries_the_minted_token_as_a_bearer() {
    let fake = fake().await;
    let clock = FakeClock::new(1_000.0);
    let client = client_with(connection(&fake.base), &clock, Duration::from_secs(5));

    client.insert(&[]).await.unwrap();

    let calls = fake.insert_calls();
    assert_eq!(calls[0].headers["authorization"], "Bearer tok-1");
}
