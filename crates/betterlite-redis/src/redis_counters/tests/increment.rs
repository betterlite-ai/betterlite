use rstest::rstest;

use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_counters::RedisCountersError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

use super::support::{counters, direct_settings, runtime, transport};

#[test]
fn increment_cache_sends_incrby_then_ttl_then_expire_for_a_key_without_ttl() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("INCRBY", Reply::Int(5));
    server.script("TTL", Reply::Int(-1));
    let (counters, hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let count = counters
        .increment_cache("counter", 3, None)
        .expect("the count");
    assert_eq!(count, 5);
    let sent: Vec<String> = server
        .commands()
        .into_iter()
        .filter(|recorded| recorded.name != "CLIENT")
        .map(|recorded| recorded.name)
        .collect();
    assert_eq!(sent, ["INCRBY", "TTL", "EXPIRE"]);
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "counter", "60"]
    );
    assert_eq!(
        server.commands_named("INCRBY")[0].args,
        ["INCRBY", "counter", "3"]
    );
    assert_eq!(
        hooks.success_names(),
        vec![
            "increment_cache",
            "increment_cache_ttl",
            "increment_cache_expire"
        ]
    );
}

#[rstest]
#[case::fractional(Some(30.9_f64), "30")]
#[case::missing(None, "60")]
#[case::not_a_number(Some(f64::NAN), "60")]
#[test]
fn increment_cache_cuts_the_requested_ttl_toward_zero(
    #[case] ttl: Option<f64>,
    #[case] expected: &str,
) {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("INCRBY", Reply::Int(1));
    server.script("TTL", Reply::Int(-1));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    counters
        .increment_cache("counter", 1, ttl)
        .expect("the count");
    let expire = server.commands_named("EXPIRE");
    assert_eq!(expire.len(), 1);
    assert_eq!(expire[0].args, ["EXPIRE", "counter", expected]);
}

#[rstest]
#[case::missing_key(-2)]
#[case::already_expiring(30)]
#[test]
fn increment_cache_sends_no_expire_when_the_ttl_is_not_minus_one(#[case] current_ttl: i64) {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("INCRBY", Reply::Int(2));
    server.script("TTL", Reply::Int(current_ttl));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let count = counters
        .increment_cache("counter", 2, None)
        .expect("the count");
    assert_eq!(count, 2);
    assert_eq!(server.commands_named("TTL").len(), 1);
    assert!(server.commands_named("EXPIRE").is_empty());
}

#[test]
fn an_infinite_ttl_is_invalid_and_sends_nothing() {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    let (counters, hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .increment_cache("counter", 1, Some(f64::INFINITY))
        .expect_err("the infinite ttl");
    assert!(matches!(error, RedisCountersError::InvalidTtl));
    assert!(server.commands().is_empty());
    assert!(hooks.failures().is_empty());
}

#[rstest]
#[case::bare("counter", "ns:counter")]
#[case::already_prefixed("ns:counter", "ns:counter")]
#[test]
fn increment_cache_namespaces_the_key_once(#[case] key: &str, #[case] expected: &str) {
    let runtime = runtime();
    let server = runtime.block_on(FakeServer::start());
    server.script("INCRBY", Reply::Int(1));
    server.script("TTL", Reply::Int(20));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    counters.increment_cache(key, 1, None).expect("the count");
    let increment = server.commands_named("INCRBY");
    assert_eq!(increment.len(), 1);
    assert_eq!(increment[0].args, ["INCRBY", expected, "1"]);
    assert_eq!(server.commands_named("TTL")[0].args, ["TTL", expected]);
}
