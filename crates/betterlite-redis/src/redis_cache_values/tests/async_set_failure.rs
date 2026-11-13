use super::support::{
    harness, harness_on_transport, harness_with_missing_credentials, harness_without_server,
    missing_tls_transport, sentinel_transport, settings,
};
use crate::redis_cache_values::error::RedisCacheValuesError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test]
async fn swallows_an_unreachable_client_after_the_failure_hook() {
    let harness = harness_without_server(settings(Some("ns"), Some(60), None));
    let stored = harness
        .values
        .async_set_cache(Some("alpha"), "cached", None, false)
        .await
        .expect("the swallowed connection failure");
    assert_eq!(stored, None);
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_set_cache:Some(\"ns:alpha\"):"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test]
async fn swallows_an_unreachable_sentinel() {
    let harness = harness_on_transport(settings(Some("ns"), Some(60), None), sentinel_transport());
    let stored = harness
        .values
        .async_set_cache(Some("alpha"), "cached", None, false)
        .await
        .expect("the swallowed sentinel failure");
    assert_eq!(stored, None);
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_set_cache:Some(\"ns:alpha\"):"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test]
async fn swallows_a_missing_tls_file() {
    let harness = harness_on_transport(
        settings(Some("ns"), Some(60), None),
        missing_tls_transport(),
    );
    let stored = harness
        .values
        .async_set_cache(Some("alpha"), "cached", None, false)
        .await
        .expect("the swallowed TLS file failure");
    assert_eq!(stored, None);
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_set_cache:Some(\"ns:alpha\"):"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

#[tokio::test]
async fn returns_a_client_construction_failure() {
    let server = FakeServer::start().await;
    let harness = harness_with_missing_credentials(&server, settings(Some("ns"), Some(60), None));
    let error = harness
        .values
        .async_set_cache(Some("alpha"), "cached", None, false)
        .await
        .expect_err("the missing credential source fails");
    assert!(matches!(error, RedisCacheValuesError::ClientReuse(_)));
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_set_cache:None:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}

#[tokio::test]
async fn swallows_a_set_failure_through_the_gate() {
    let server = FakeServer::start().await;
    server.script("SET", Reply::Error("ERR broken".to_owned()));
    let harness = harness(&server, settings(Some("ns"), Some(60), None));
    let stored = harness
        .values
        .async_set_cache(Some("alpha"), "cached", None, false)
        .await
        .expect("the swallowed failure");
    assert_eq!(stored, None);
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_set_cache:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}
