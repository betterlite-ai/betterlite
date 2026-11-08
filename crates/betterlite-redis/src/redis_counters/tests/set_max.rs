use rstest::rstest;

use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_counters::RedisCountersError;
use crate::redis_counters::set_max::SET_MAX_LUA;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

use super::support::{ADMISSION, DEFAULT_TTL, counters, direct_settings, transport};

#[tokio::test]
async fn async_set_max_sends_the_max_script_with_the_value_and_ttl_as_text() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Bulk("5.5".to_owned()));
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    let result = counters
        .async_set_max("limit", 5.5_f64, Some(120))
        .await
        .expect("the max");
    assert_eq!(result, Some(5.5_f64));
    assert_eq!(
        server.commands_named("EVAL")[0].args,
        ["EVAL", SET_MAX_LUA, "1", "ns:limit", "5.5", "120"]
    );
    assert_eq!(gate.enters(), vec!["async_set_max"]);
    assert_eq!(gate.exits(), vec![ADMISSION]);
}

#[tokio::test]
async fn async_set_max_uses_the_default_ttl_when_none_is_given() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Bulk("1".to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .async_set_max("limit", 1.0_f64, None)
        .await
        .expect("the max");
    let expected = vec![
        "EVAL".to_owned(),
        SET_MAX_LUA.to_owned(),
        "1".to_owned(),
        "limit".to_owned(),
        "1".to_owned(),
        DEFAULT_TTL.to_string(),
    ];
    assert_eq!(server.commands_named("EVAL")[0].args, expected);
}

#[tokio::test]
async fn async_set_max_sends_a_zero_ttl_only_when_the_caller_passes_it() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Bulk("1".to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .async_set_max("limit", 1.0_f64, Some(0))
        .await
        .expect("the max");
    assert_eq!(
        server.commands_named("EVAL")[0].args,
        ["EVAL", SET_MAX_LUA, "1", "limit", "1", "0"]
    );
}

#[tokio::test]
async fn async_set_max_returns_none_for_a_nil_reply() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Nil);
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let result = counters
        .async_set_max("limit", 2.0_f64, Some(30))
        .await
        .expect("the max");
    assert_eq!(result, None);
}

#[tokio::test]
async fn async_set_max_rejects_a_bulk_reply_that_is_not_a_number() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Bulk("not-a-number".to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .async_set_max("limit", 2.0_f64, Some(30))
        .await
        .expect_err("the unparsable reply");
    assert!(matches!(
        error,
        RedisCountersError::UnexpectedReply {
            operation: "async_set_max"
        }
    ));
}

#[rstest]
#[case::not_a_number(f64::NAN)]
#[case::positive_infinity(f64::INFINITY)]
#[case::negative_infinity(f64::NEG_INFINITY)]
#[tokio::test]
async fn async_set_max_rejects_a_non_finite_value_before_sending(#[case] value: f64) {
    let server = FakeServer::start().await;
    let (counters, _hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .async_set_max("limit", value, Some(30))
        .await
        .expect_err("the non-finite value");
    assert!(matches!(error, RedisCountersError::NonFiniteValue));
    assert!(server.commands().is_empty());
    assert_eq!(gate.failures().len(), 1);
    assert!(gate.exits().is_empty());
}

#[tokio::test]
async fn async_set_max_passes_a_script_error_through_unchanged() {
    let server = FakeServer::start().await;
    server.script(
        "EVAL",
        Reply::Error("ERR user_script:1: attempt to compare".to_owned()),
    );
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .async_set_max("limit", 2.0_f64, Some(30))
        .await
        .expect_err("the script error");
    assert!(error.to_string().contains("attempt to compare"), "{error}");
}
