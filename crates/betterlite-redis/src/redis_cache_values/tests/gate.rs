use super::support::{harness, harness_with_open_gate};
use crate::redis_cache_values::error::RedisCacheValuesError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test]
async fn an_open_breaker_rejects_every_guarded_write_without_a_command() {
    let server = FakeServer::start().await;
    let harness = harness_with_open_gate(&server, Some("ns"), Some(60), Some(2));
    let values = &harness.values;
    let single = values
        .async_set_cache(Some("alpha"), "cached", None, false)
        .await;
    assert!(matches!(single, Err(RedisCacheValuesError::BreakerOpen(_))));
    let pipeline = values
        .async_set_cache_pipeline(&[("alpha".to_owned(), "first")], None)
        .await;
    assert!(matches!(
        pipeline,
        Err(RedisCacheValuesError::BreakerOpen(_))
    ));
    let per_entry = values
        .async_set_cache_pipeline_with_ttls(&[("alpha".to_owned(), "first", None)])
        .await;
    assert!(matches!(
        per_entry,
        Err(RedisCacheValuesError::BreakerOpen(_))
    ));
    let sadd = values
        .async_set_cache_sadd("members", &["one".to_owned()], None)
        .await;
    assert!(matches!(sadd, Err(RedisCacheValuesError::BreakerOpen(_))));
    let buffered = values.batch_cache_write("alpha", "cached").await;
    assert!(matches!(
        buffered,
        Err(RedisCacheValuesError::BreakerOpen(_))
    ));
    assert!(server.commands().is_empty());
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn an_open_breaker_rejects_both_reads_without_a_command() {
    let server = FakeServer::start().await;
    let harness = harness_with_open_gate(&server, Some("ns"), Some(60), None);
    let single = harness.values.get_cache("alpha");
    assert!(matches!(single, Err(RedisCacheValuesError::BreakerOpen(_))));
    let batch = harness.values.batch_get_cache(&[Some("alpha")]);
    assert!(matches!(batch, Err(RedisCacheValuesError::BreakerOpen(_))));
    assert!(server.commands().is_empty());
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn a_success_exits_the_gate_once() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"), Some(60), None);
    harness
        .values
        .async_set_cache(Some("alpha"), "cached", None, false)
        .await
        .expect("the write");
    assert_eq!(
        harness.gate.entries(),
        vec!["enter:async_set_cache".to_owned(), "exit:1".to_owned()]
    );
}

#[tokio::test]
async fn a_swallowed_failure_is_recorded_before_the_exit() {
    let server = FakeServer::start().await;
    server.script("SET", Reply::Error("ERR broken".to_owned()));
    let harness = harness(&server, Some("ns"), Some(60), None);
    harness
        .values
        .async_set_cache(Some("alpha"), "cached", None, false)
        .await
        .expect("the swallowed failure");
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}
