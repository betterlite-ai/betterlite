use std::sync::Arc;
use std::time::Duration;

use super::push_server::PushServer;
use super::support::{redis_cache, redis_cache_at};
use crate::config_sync::endpoint::{ConfigSyncPubSubClient as _, ConfigSyncSubscription as _};
use crate::config_sync::publish::publish_config_change;
use crate::config_sync::redis_endpoint::RedisConfigSyncClient;
use crate::redis_live_client::tests::fake_server::FakeServer;

#[tokio::test]
async fn the_redis_endpoint_subscribes_to_the_default_channel() {
    let server = FakeServer::start().await;
    let cache = redis_cache(&server, None).await;
    let client = RedisConfigSyncClient {
        cache: Arc::new(cache),
    };
    let mut subscription = client.subscribe().await.expect("a subscription");
    let subscribed = server.commands_named("SUBSCRIBE");
    assert_eq!(subscribed.len(), 1);
    assert_eq!(
        subscribed[0].args,
        ["SUBSCRIBE", "litellm_proxy.config_change"]
    );
    let empty = subscription
        .next_message(Duration::from_millis(20))
        .await
        .expect("an empty wait");
    assert!(empty.is_none());
    subscription.close().await;
    let closed = subscription
        .next_message(Duration::from_millis(20))
        .await
        .expect("a closed subscription");
    assert!(closed.is_none());
}

#[tokio::test]
async fn the_redis_endpoint_subscribes_to_the_namespaced_channel() {
    let server = FakeServer::start().await;
    let cache = redis_cache(&server, Some("blue")).await;
    let client = RedisConfigSyncClient {
        cache: Arc::new(cache),
    };
    let _subscription = client.subscribe().await.expect("a subscription");
    let subscribed = server.commands_named("SUBSCRIBE");
    assert_eq!(subscribed.len(), 1);
    assert_eq!(
        subscribed[0].args,
        ["SUBSCRIBE", "blue:litellm_proxy.config_change"]
    );
}

#[tokio::test]
async fn the_redis_endpoint_delivers_a_published_message() {
    let server = PushServer::start().await;
    let cache = Arc::new(redis_cache_at(&server.host(), server.port(), None).await);
    let client = RedisConfigSyncClient {
        cache: Arc::clone(&cache),
    };
    let mut subscription = client.subscribe().await.expect("a subscription");
    publish_config_change(Some(&cache), "litellm_proxymodeltable").await;
    let message = subscription
        .next_message(Duration::from_secs(2))
        .await
        .expect("a message or a timeout")
        .expect("the published message");
    assert_eq!(message.channel, "litellm_proxy.config_change");
    assert_eq!(
        message.payload,
        br#"{"object_type":"litellm_proxymodeltable"}"#.to_vec()
    );
}

#[tokio::test]
async fn a_closed_subscription_no_longer_reaches_redis() {
    let server = PushServer::start().await;
    let cache = redis_cache_at(&server.host(), server.port(), None).await;
    let client = RedisConfigSyncClient {
        cache: Arc::new(cache),
    };
    let mut subscription = client.subscribe().await.expect("a subscription");
    subscription.close().await;
    server.close_connections();
    let outcome = subscription.next_message(Duration::from_secs(2)).await;
    assert!(matches!(outcome, Ok(None)));
}

#[tokio::test]
async fn a_closed_redis_connection_is_an_error() {
    let server = PushServer::start().await;
    let cache = redis_cache_at(&server.host(), server.port(), None).await;
    let client = RedisConfigSyncClient {
        cache: Arc::new(cache),
    };
    let mut subscription = client.subscribe().await.expect("a subscription");
    server.close_connections();
    let outcome = subscription.next_message(Duration::from_secs(2)).await;
    assert!(outcome.is_err());
}
