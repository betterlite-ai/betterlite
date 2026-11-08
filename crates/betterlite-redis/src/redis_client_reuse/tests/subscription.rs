use std::time::{Duration, Instant};

use secrecy::SecretString;

use super::fixtures::pubsub_server::PubSubServer;
use super::fixtures::{direct_settings, store, transport};
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_live_client::tests::fake_server::FakeServer;

#[tokio::test]
async fn publish_returns_the_number_of_subscribers() {
    let server = PubSubServer::start().await;
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let publisher = transport.pubsub_client().await.expect("a publisher");
    assert_eq!(
        publisher
            .publish("channel", "before")
            .await
            .expect("a publish"),
        0
    );
    let subscriber = transport.pubsub_client().await.expect("a subscriber");
    let _subscription = subscriber
        .subscribe(&["channel".to_owned()])
        .await
        .expect("a subscription");
    assert_eq!(
        publisher
            .publish("channel", "after")
            .await
            .expect("a publish"),
        1
    );
}

#[tokio::test]
async fn subscribe_delivers_a_published_message() {
    let server = PubSubServer::start().await;
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let publisher = transport.pubsub_client().await.expect("a publisher");
    let subscriber = transport.pubsub_client().await.expect("a subscriber");
    let mut subscription = subscriber
        .subscribe(&["channel".to_owned()])
        .await
        .expect("a subscription");
    publisher
        .publish("channel", "payload")
        .await
        .expect("a publish");
    let message = subscription
        .next_message(Duration::from_secs(2))
        .await
        .expect("a message")
        .expect("a published message, not a confirmation");
    assert_eq!(message.channel, "channel");
    assert_eq!(message.payload, b"payload");
}

#[tokio::test]
async fn next_message_returns_none_when_the_wait_ends() {
    let server = PubSubServer::start().await;
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let subscriber = transport.pubsub_client().await.expect("a subscriber");
    let mut subscription = subscriber
        .subscribe(&["silent".to_owned()])
        .await
        .expect("a subscription");
    let message = subscription
        .next_message(Duration::from_millis(100))
        .await
        .expect("the empty wait");
    assert!(message.is_none());
}

#[tokio::test]
async fn a_connect_that_never_answers_times_out() {
    let server = FakeServer::start().await;
    let store = store();
    let mut settings = direct_settings(&server.host(), server.port());
    settings.password = Some(SecretString::from("token".to_owned()));
    settings.socket_connect_timeout = Some(0.2_f64);
    settings.socket_timeout = Some(0.2_f64);
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let pubsub = transport.pubsub_client().await.expect("a pubsub client");
    server.mute();
    let started = Instant::now();
    let error = pubsub
        .subscribe(&["channel".to_owned()])
        .await
        .err()
        .expect("the muted server never confirms");
    assert!(matches!(error, ClientReuseError::Timeout(_)));
    assert!(started.elapsed() < Duration::from_secs(3));
}
