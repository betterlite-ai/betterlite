use super::fixtures::{client, direct_settings, factory};
use crate::redis_client_factories::RedisClient;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_connection_settings::model::settings::ClientKind;
use crate::redis_live_client::tests::fake_server::FakeServer;
use crate::redis_live_client::{LiveClientError, LiveClientFactory};

#[tokio::test]
async fn concurrent_callers_of_one_key_open_one_connection() {
    let server = FakeServer::start().await;
    let store = RedisClientStore::new();
    let factory = factory();
    let client = client(
        &direct_settings(&server.host(), server.port()),
        ClientKind::Async,
    );
    let (left, right) = tokio::join!(
        store.get_or_open("shared", &factory, &client, None),
        store.get_or_open("shared", &factory, &client, None),
    );
    left.expect("the first concurrent client");
    right.expect("the second concurrent client");
    assert_eq!(server.connections(), 1);
}

#[tokio::test]
async fn later_callers_share_the_connection() {
    let server = FakeServer::start().await;
    let store = RedisClientStore::new();
    let factory = factory();
    let client = client(
        &direct_settings(&server.host(), server.port()),
        ClientKind::Async,
    );
    let first = store
        .get_or_open("shared", &factory, &client, None)
        .await
        .expect("the first client");
    let second = store
        .get_or_open("shared", &factory, &client, None)
        .await
        .expect("the second client");
    assert_eq!(server.connections(), 1);
    assert_eq!(
        first
            .query::<String>(&redis::cmd("PING"))
            .await
            .expect("the first ping"),
        "PONG"
    );
    assert_eq!(
        second
            .query::<String>(&redis::cmd("PING"))
            .await
            .expect("the second ping"),
        "PONG"
    );
}

#[tokio::test]
async fn a_failed_open_leaves_the_cell_for_the_next_try() {
    let server = FakeServer::start().await;
    let store = RedisClientStore::new();
    let factory = factory();
    let settings = direct_settings(&server.host(), server.port());
    let mut missing_settings = settings.clone();
    missing_settings.credential_provider = true;
    let missing_credentials = client(&missing_settings, ClientKind::Async);
    let error = store
        .get_or_open("shared", &factory, &missing_credentials, None)
        .await
        .err()
        .expect("a credential source is missing");
    assert!(matches!(
        error,
        ClientReuseError::Live(LiveClientError::CredentialSourceMissing)
    ));
    store
        .get_or_open(
            "shared",
            &factory,
            &client(&settings, ClientKind::Async),
            None,
        )
        .await
        .expect("the retry opens");
    assert_eq!(server.connections(), 1);
}

async fn open_many(
    store: &RedisClientStore,
    factory: &LiveClientFactory,
    client: &RedisClient,
    count: u16,
) {
    for index in 0..count {
        store
            .get_or_open(&format!("key-{index}"), factory, client, None)
            .await
            .expect("an opened client");
    }
}

#[tokio::test]
async fn the_oldest_entry_is_evicted_past_two_hundred() {
    let server = FakeServer::start().await;
    let store = RedisClientStore::new();
    let factory = factory();
    let client = client(
        &direct_settings(&server.host(), server.port()),
        ClientKind::Async,
    );
    open_many(&store, &factory, &client, 201).await;
    let after_inserts = server.connections();
    store
        .get_or_open("key-200", &factory, &client, None)
        .await
        .expect("the newest entry is still stored");
    assert_eq!(server.connections(), after_inserts);
    store
        .get_or_open("key-0", &factory, &client, None)
        .await
        .expect("the oldest entry reopens");
    assert!(
        server.connections() > after_inserts,
        "the oldest entry was evicted"
    );
}

#[tokio::test]
async fn remove_drops_the_entry() {
    let server = FakeServer::start().await;
    let store = RedisClientStore::new();
    let factory = factory();
    let client = client(
        &direct_settings(&server.host(), server.port()),
        ClientKind::Async,
    );
    store
        .get_or_open("shared", &factory, &client, None)
        .await
        .expect("the first client");
    let opened = server.connections();
    store.remove("shared");
    store
        .get_or_open("shared", &factory, &client, None)
        .await
        .expect("the reopened client");
    assert!(server.connections() > opened);
}
