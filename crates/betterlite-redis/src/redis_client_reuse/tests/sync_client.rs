use std::sync::Arc;
use std::time::{Duration, Instant};

use redis::cmd;

use super::fixtures::{credential, direct_settings, source, store, transport};
use crate::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::tests::fake_server::FakeServer;

fn ping(transport: &RedisTransport) -> String {
    transport
        .sync_client
        .query(&cmd("PING"))
        .expect("a ping reply")
}

#[tokio::test]
async fn the_connection_is_reused_across_calls() {
    let server = FakeServer::start().await;
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    assert_eq!(ping(&transport), "PONG");
    assert_eq!(ping(&transport), "PONG");
    assert_eq!(ping(&transport), "PONG");
    assert_eq!(server.connections(), 1);
}

#[tokio::test]
async fn a_dropped_connection_is_reopened_with_a_fresh_credential() {
    let server = FakeServer::start().await;
    server.close_after("GET");
    let mut settings = direct_settings(&server.host(), server.port());
    settings.credential_provider = true;
    let factory = LiveClientFactory::new(Some(source(vec![
        credential(None, "first-token"),
        credential(None, "second-token"),
    ])));
    let transport = RedisTransport::new(
        settings,
        RedisCacheFamily::Standalone,
        Arc::new(factory),
        store(),
        None,
    )
    .expect("a transport");
    let first: String = transport
        .sync_client
        .query(cmd("GET").arg("key"))
        .expect("the first read");
    assert_eq!(first, "OK");
    assert!(
        transport
            .sync_client
            .query::<String>(cmd("GET").arg("key"))
            .is_err(),
        "the closed connection fails"
    );
    let reopened: String = transport
        .sync_client
        .query(cmd("GET").arg("key"))
        .expect("the reopened read");
    assert_eq!(reopened, "OK");
    assert_eq!(server.connections(), 2);
    let auth = server.commands_named("AUTH");
    assert!(
        auth.iter()
            .any(|recorded| recorded.args.contains(&"second-token".to_owned())),
        "the reopen fetched a fresh credential"
    );
}

#[tokio::test]
async fn a_silent_server_fails_at_the_response_timeout() {
    let server = FakeServer::start().await;
    server.silence("GET");
    let store = store();
    let mut settings = direct_settings(&server.host(), server.port());
    settings.socket_timeout = Some(0.2_f64);
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let started = Instant::now();
    let error = transport.sync_client.query::<String>(cmd("GET").arg("key"));
    assert!(error.is_err(), "the silent server never answers");
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(150), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
}
