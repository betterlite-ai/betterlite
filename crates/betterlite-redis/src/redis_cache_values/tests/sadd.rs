use super::support::{harness, harness_without_server};
use crate::redis_cache_values::error::RedisCacheValuesError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn members() -> Vec<String> {
    vec!["one".to_owned(), "two".to_owned()]
}

#[tokio::test]
async fn sends_sadd_then_expire_when_a_ttl_resolves() {
    let server = FakeServer::start().await;
    server.script("SADD", Reply::Int(2));
    server.script("EXPIRE", Reply::Int(1));
    let harness = harness(&server, Some("ns"), None, None);
    harness
        .values
        .async_set_cache_sadd("members", &members(), Some(45.0_f64))
        .await
        .expect("the set write");
    assert_eq!(
        server.commands_named("SADD")[0].args,
        ["SADD", "ns:members", "one", "two"]
    );
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "ns:members", "45"]
    );
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:async_set_cache_sadd:None".to_owned()]
    );
}

#[tokio::test]
async fn sends_only_sadd_when_no_ttl_resolves() {
    let server = FakeServer::start().await;
    server.script("SADD", Reply::Int(2));
    let harness = harness(&server, Some("ns"), None, None);
    harness
        .values
        .async_set_cache_sadd("members", &members(), None)
        .await
        .expect("the set write");
    assert_eq!(server.commands_named("SADD").len(), 1);
    assert!(server.commands_named("EXPIRE").is_empty());
}

#[tokio::test]
async fn uses_the_default_ttl_when_the_entry_has_none() {
    let server = FakeServer::start().await;
    server.script("SADD", Reply::Int(2));
    server.script("EXPIRE", Reply::Int(1));
    let harness = harness(&server, None, Some(60), None);
    harness
        .values
        .async_set_cache_sadd("members", &members(), None)
        .await
        .expect("the set write");
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "members", "60"]
    );
}

#[tokio::test]
async fn returns_a_client_failure() {
    let harness = harness_without_server(Some("ns"), None);
    let error = harness
        .values
        .async_set_cache_sadd("members", &members(), None)
        .await
        .expect_err("the unreachable client fails");
    assert!(matches!(error, RedisCacheValuesError::ClientReuse(_)));
    let hooks = harness.hooks.entries();
    assert!(hooks[0].starts_with("failure:async_set_cache_sadd:"));
}

#[tokio::test]
async fn swallows_a_sadd_failure() {
    let server = FakeServer::start().await;
    server.script("SADD", Reply::Error("ERR broken".to_owned()));
    let harness = harness(&server, Some("ns"), None, None);
    harness
        .values
        .async_set_cache_sadd("members", &members(), Some(45.0_f64))
        .await
        .expect("the swallowed failure");
    assert_eq!(server.commands_named("SADD").len(), 1);
    assert!(server.commands_named("EXPIRE").is_empty());
    assert!(
        harness
            .hooks
            .entries()
            .iter()
            .any(|entry| entry.starts_with("failure:async_set_cache_sadd:"))
    );
    assert_eq!(
        harness.gate.entries()[1].split(':').next(),
        Some("swallowed")
    );
}

#[tokio::test]
async fn swallows_an_expire_failure_after_the_set_is_written() {
    let server = FakeServer::start().await;
    server.script("SADD", Reply::Int(2));
    server.script("EXPIRE", Reply::Error("ERR broken".to_owned()));
    let harness = harness(&server, Some("ns"), None, None);
    harness
        .values
        .async_set_cache_sadd("members", &members(), Some(45.0_f64))
        .await
        .expect("the swallowed failure");
    assert_eq!(server.commands_named("SADD").len(), 1);
    assert_eq!(server.commands_named("EXPIRE").len(), 1);
    assert!(
        harness
            .gate
            .entries()
            .iter()
            .any(|entry| entry.starts_with("swallowed:1:"))
    );
}

#[tokio::test]
async fn sends_sadd_with_no_members_and_swallows_the_server_error() {
    let server = FakeServer::start().await;
    server.script(
        "SADD",
        Reply::Error("ERR wrong number of arguments".to_owned()),
    );
    let harness = harness(&server, Some("ns"), None, None);
    harness
        .values
        .async_set_cache_sadd("members", &[], Some(45.0_f64))
        .await
        .expect("the swallowed failure");
    assert_eq!(
        server.commands_named("SADD")[0].args,
        ["SADD", "ns:members"]
    );
    assert!(server.commands_named("EXPIRE").is_empty());
}
