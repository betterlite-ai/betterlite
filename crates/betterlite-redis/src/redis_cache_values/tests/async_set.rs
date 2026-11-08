use super::support::harness;
use crate::redis_cache_values::error::RedisCacheValuesError;
use crate::redis_client_reuse::breaker::BreakerOpen;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test]
async fn sends_a_namespaced_set_with_the_json_text_and_the_default_ttl() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"), Some(60), None);
    let stored = harness
        .values
        .async_set_cache(Some("alpha"), "cached", None, false)
        .await
        .expect("the stored value");
    assert_eq!(stored, Some(true));
    let sets = server.commands_named("SET");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].args, ["SET", "ns:alpha", "\"cached\"", "EX", "60"]);
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:async_set_cache:Some(\"ns:alpha\")".to_owned()]
    );
}

#[tokio::test]
async fn adds_nx_only_when_it_is_asked_for() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"), Some(60), None);
    harness
        .values
        .async_set_cache(Some("alpha"), "cached", Some(5.9_f64), true)
        .await
        .expect("the conditional write");
    let sets = server.commands_named("SET");
    assert_eq!(
        sets[0].args,
        ["SET", "ns:alpha", "\"cached\"", "NX", "EX", "5"]
    );
}

#[tokio::test]
async fn a_nil_reply_means_the_key_already_existed() {
    let server = FakeServer::start().await;
    server.script("SET", Reply::Nil);
    let harness = harness(&server, None, None, None);
    let stored = harness
        .values
        .async_set_cache(Some("alpha"), "cached", None, true)
        .await
        .expect("the conditional write");
    assert_eq!(stored, None);
}

#[tokio::test]
async fn sends_a_zero_ttl_as_given() {
    let server = FakeServer::start().await;
    let harness = harness(&server, None, Some(60), None);
    harness
        .values
        .async_set_cache(Some("alpha"), "cached", Some(0.0_f64), false)
        .await
        .expect("the write");
    assert_eq!(
        server.commands_named("SET")[0].args,
        ["SET", "alpha", "\"cached\"", "EX", "0"]
    );
}

#[tokio::test]
async fn a_none_key_sends_nothing() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"), Some(60), None);
    let stored = harness
        .values
        .async_set_cache(None, "cached", None, false)
        .await
        .expect("the skipped write");
    assert_eq!(stored, None);
    assert!(server.commands().is_empty());
    assert_eq!(server.connections(), 0);
    assert!(harness.hooks.entries().is_empty());
    assert_eq!(
        harness.gate.entries(),
        vec!["enter:async_set_cache".to_owned(), "exit:1".to_owned()]
    );
}

#[tokio::test]
async fn an_unexpected_reply_is_still_a_stored_value() {
    let server = FakeServer::start().await;
    server.script("SET", Reply::Simple("LOOKS-ODD".to_owned()));
    let harness = harness(&server, None, None, None);
    let stored = harness
        .values
        .async_set_cache(Some("alpha"), "cached", None, false)
        .await
        .expect("the write");
    assert_eq!(stored, Some(true));
}

#[test]
fn the_breaker_error_names_the_method() {
    let open = BreakerOpen {
        name: "async_set_cache",
    };
    let error = RedisCacheValuesError::from(open);
    assert_eq!(
        error.to_string(),
        "Redis circuit breaker is open, skipping async_set_cache"
    );
}
