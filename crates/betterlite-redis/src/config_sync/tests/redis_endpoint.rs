use std::sync::Arc;
use std::time::Duration;

use super::support::redis_cache;
use crate::config_sync::endpoint::{ConfigSyncPubSubClient as _, ConfigSyncSubscription as _};
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
