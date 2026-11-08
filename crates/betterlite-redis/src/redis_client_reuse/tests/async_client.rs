use secrecy::SecretString;

use super::fixtures::{direct_settings, store, transport};
use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_live_client::tests::fake_server::FakeServer;

#[tokio::test]
async fn equal_settings_share_one_store_entry_across_transports() {
    let server = FakeServer::start().await;
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let first = transport(settings.clone(), RedisCacheFamily::Standalone, &store, None);
    let second = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let left = first.async_client().await.expect("the first client");
    let right = second.async_client().await.expect("the second client");
    assert_eq!(server.connections(), 1);
    assert_eq!(
        left.query::<String>(&redis::cmd("PING"))
            .await
            .expect("the first ping"),
        "PONG"
    );
    assert_eq!(
        right
            .query::<String>(&redis::cmd("PING"))
            .await
            .expect("the second ping"),
        "PONG"
    );
}

#[tokio::test]
async fn different_passwords_open_two_connections() {
    let server = FakeServer::start().await;
    let store = store();
    let mut first_settings = direct_settings(&server.host(), server.port());
    first_settings.password = Some(SecretString::from("first-password".to_owned()));
    let mut second_settings = direct_settings(&server.host(), server.port());
    second_settings.password = Some(SecretString::from("second-password".to_owned()));
    let first = transport(first_settings, RedisCacheFamily::Standalone, &store, None);
    let second = transport(second_settings, RedisCacheFamily::Standalone, &store, None);
    first.async_client().await.expect("the first client");
    second.async_client().await.expect("the second client");
    assert_eq!(server.connections(), 2);
    let auth = server.commands_named("AUTH");
    assert_eq!(auth.len(), 2);
    assert_eq!(auth[0].args, ["AUTH", "first-password"]);
    assert_eq!(auth[1].args, ["AUTH", "second-password"]);
}

#[tokio::test]
async fn the_async_client_serves_redis_commands_and_a_pipeline() {
    let server = FakeServer::start().await;
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let client = transport.async_client().await.expect("the async client");
    assert!(
        client
            .ping()
            .await
            .expect("the ping through the command trait")
    );
    let _pipeline = client.pipeline(false);
}
