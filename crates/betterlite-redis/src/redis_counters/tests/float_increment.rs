use rstest::rstest;

use crate::cache_primitives::pipeline_increment::IncrementOperation;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_counters::RedisCountersError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

use super::support::{counters, direct_settings, transport};

fn sent_commands(server: &FakeServer) -> Vec<(String, Vec<String>)> {
    server
        .commands()
        .into_iter()
        .filter(|recorded| recorded.name != "CLIENT")
        .map(|recorded| (recorded.name, recorded.args))
        .collect()
}

fn sent_names(server: &FakeServer) -> Vec<String> {
    sent_commands(server)
        .into_iter()
        .map(|(name, _args)| name)
        .collect()
}

fn increment(key: &str, value: f64, ttl: Option<i64>) -> IncrementOperation {
    IncrementOperation {
        key: key.to_owned(),
        increment_value: value,
        ttl,
    }
}

#[tokio::test]
async fn incr_by_float_with_ttl_sends_incrbyfloat_alone_without_a_ttl() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    let result = counters
        .incr_by_float_with_ttl("ns:key", 1.5_f64, None, false)
        .await
        .expect("the float");
    assert_eq!(Some(result), Some(1.5_f64));
    assert_eq!(sent_names(&server), ["INCRBYFLOAT"]);
    assert_eq!(
        server.commands_named("INCRBYFLOAT")[0].args,
        ["INCRBYFLOAT", "ns:key", "1.5"]
    );
}

#[tokio::test]
async fn incr_by_float_with_ttl_refreshes_the_expiry_in_the_pipeline() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("2.5".to_owned()));
    server.script("EXPIRE", Reply::Int(1));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let result = counters
        .incr_by_float_with_ttl("key", 2.5_f64, Some(30), true)
        .await
        .expect("the float");
    assert_eq!(Some(result), Some(2.5_f64));
    assert_eq!(sent_names(&server), ["INCRBYFLOAT", "EXPIRE"]);
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "key", "30"]
    );
    assert!(server.commands_named("TTL").is_empty());
}

#[rstest]
#[case::still_expiring(20, false)]
#[case::without_expiry(-1, true)]
#[tokio::test]
async fn incr_by_float_with_ttl_expires_only_when_ttl_answered_minus_one(
    #[case] ttl_reply: i64,
    #[case] expired: bool,
) {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("3.5".to_owned()));
    server.script("TTL", Reply::Int(ttl_reply));
    if expired {
        server.script("EXPIRE", Reply::Int(1));
    }
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let result = counters
        .incr_by_float_with_ttl("key", 3.5_f64, Some(30), false)
        .await
        .expect("the float");
    assert_eq!(Some(result), Some(3.5_f64));
    let expected = if expired {
        vec![
            "INCRBYFLOAT".to_owned(),
            "TTL".to_owned(),
            "EXPIRE".to_owned(),
        ]
    } else {
        vec!["INCRBYFLOAT".to_owned(), "TTL".to_owned()]
    };
    assert_eq!(sent_names(&server), expected);
}

#[tokio::test]
async fn incr_by_float_with_ttl_returns_the_first_pipeline_error() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Error("ERR float failed".to_owned()));
    server.script("TTL", Reply::Int(20));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .incr_by_float_with_ttl("key", 1.0_f64, Some(30), false)
        .await
        .expect_err("the pipeline error");
    assert!(error.to_string().contains("float failed"), "{error}");
}

#[tokio::test]
async fn incr_by_float_with_ttl_reports_a_non_float_reply() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Nil);
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .incr_by_float_with_ttl("key", 1.0_f64, None, false)
        .await
        .expect_err("the nil reply");
    assert!(matches!(
        error,
        RedisCountersError::UnexpectedReply {
            operation: "incr_by_float_with_ttl"
        }
    ));
}

#[tokio::test]
async fn pipeline_increment_sends_one_pipeline_in_order_and_keeps_the_floats() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    server.script("INCRBYFLOAT", Reply::Bulk("2.5".to_owned()));
    server.script("EXPIRE", Reply::Int(1));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    let operations = vec![
        increment("alpha", 1.5_f64, Some(10)),
        increment("beta", 2.5_f64, None),
    ];
    let floats = counters
        .pipeline_increment(&operations)
        .await
        .expect("the floats");
    assert_eq!(floats, vec![1.5_f64, 2.5_f64]);
    assert_eq!(
        sent_names(&server),
        ["INCRBYFLOAT", "EXPIRE", "INCRBYFLOAT"]
    );
    assert_eq!(
        server.commands_named("INCRBYFLOAT")[0].args,
        ["INCRBYFLOAT", "ns:alpha", "1.5"]
    );
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "ns:alpha", "10"]
    );
    assert_eq!(
        server.commands_named("INCRBYFLOAT")[1].args,
        ["INCRBYFLOAT", "ns:beta", "2.5"]
    );
}

#[tokio::test]
async fn an_empty_pipeline_increment_sends_nothing() {
    let server = FakeServer::start().await;
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let floats = counters.pipeline_increment(&[]).await.expect("the floats");
    assert!(floats.is_empty());
    assert!(server.commands().is_empty());
}
