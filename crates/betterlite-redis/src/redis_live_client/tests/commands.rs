use redis::Value;
use rstest::rstest;

use super::fake_server::{FakeServer, Reply};
use super::fixtures::connected;
use crate::cache_primitives::protocol::{AsyncRedisCommands as _, RedisScalar};

#[tokio::test]
async fn ping_reports_the_server_is_alive() {
    let server = FakeServer::start().await;
    let client = connected(&server).await;
    assert!(client.ping().await.expect("a reply"));
    assert_eq!(server.commands_named("PING")[0].args, ["PING"]);
}

#[tokio::test]
async fn delete_sends_del_and_returns_the_count() {
    let server = FakeServer::start().await;
    server.script("DEL", Reply::Int(2));
    let client = connected(&server).await;
    assert_eq!(client.delete(&["alpha", "beta"]).await.expect("a count"), 2);
    assert_eq!(
        server.commands_named("DEL")[0].args,
        ["DEL", "alpha", "beta"]
    );
}

#[tokio::test]
async fn ttl_sends_ttl_and_returns_the_seconds() {
    let server = FakeServer::start().await;
    server.script("TTL", Reply::Int(7));
    let client = connected(&server).await;
    assert_eq!(client.ttl("alpha").await.expect("a ttl"), 7);
    assert_eq!(server.commands_named("TTL")[0].args, ["TTL", "alpha"]);
}

#[rstest]
#[case::expires(1, true)]
#[case::kept(0, false)]
#[tokio::test]
async fn expire_maps_the_reply_to_a_bool(#[case] reply: i64, #[case] expected: bool) {
    let server = FakeServer::start().await;
    server.script("EXPIRE", Reply::Int(reply));
    let client = connected(&server).await;
    assert_eq!(client.expire("alpha", 60).await.expect("a reply"), expected);
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "alpha", "60"]
    );
}

#[tokio::test]
async fn rpush_sends_every_scalar_as_it_is() {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Int(3));
    let client = connected(&server).await;
    let values = [
        RedisScalar::Text("one".to_owned()),
        RedisScalar::Bytes(vec![b'2']),
        RedisScalar::Float(1.5),
    ];
    assert_eq!(client.rpush("list", &values).await.expect("a length"), 3);
    assert_eq!(
        server.commands_named("RPUSH")[0].args,
        ["RPUSH", "list", "one", "2", "1.5"]
    );
}

#[tokio::test]
async fn lpop_without_a_count_sends_no_count() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Bulk("value".to_owned()));
    let client = connected(&server).await;
    let reply = client.lpop("list", None).await.expect("a reply");
    assert_eq!(reply, Value::BulkString(b"value".to_vec()));
    assert_eq!(server.commands_named("LPOP")[0].args, ["LPOP", "list"]);
}

#[tokio::test]
async fn lpop_with_a_count_sends_it_and_returns_the_array() {
    let server = FakeServer::start().await;
    server.script(
        "LPOP",
        Reply::Array(vec![
            Reply::Bulk("first".to_owned()),
            Reply::Bulk("second".to_owned()),
        ]),
    );
    let client = connected(&server).await;
    let reply = client.lpop("list", Some(2)).await.expect("a reply");
    assert_eq!(
        reply,
        Value::Array(vec![
            Value::BulkString(b"first".to_vec()),
            Value::BulkString(b"second".to_vec()),
        ])
    );
    assert_eq!(server.commands_named("LPOP")[0].args, ["LPOP", "list", "2"]);
}

#[tokio::test]
async fn eval_sends_the_script_numkeys_and_arguments() {
    let server = FakeServer::start().await;
    server.script("EVAL", Reply::Int(7));
    let client = connected(&server).await;
    let arguments = [
        RedisScalar::Text("key".to_owned()),
        RedisScalar::Bytes(b"1".to_vec()),
        RedisScalar::Float(2.5),
        RedisScalar::Text("h\u{e9}llo".to_owned()),
    ];
    let reply = client
        .eval("return redis.call('GET', KEYS[1])", 1, &arguments)
        .await
        .expect("a reply");
    assert_eq!(reply, Value::Int(7));
    assert_eq!(
        server.commands_named("EVAL")[0].args,
        [
            "EVAL",
            "return redis.call('GET', KEYS[1])",
            "1",
            "key",
            "1",
            "2.5",
            "h\u{e9}llo",
        ]
    );
}

#[tokio::test]
async fn a_nil_reply_stays_nil() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Nil);
    let client = connected(&server).await;
    let reply = client.lpop("list", None).await.expect("a reply");
    assert_eq!(reply, Value::Nil);
}

#[tokio::test]
async fn a_simple_string_reply_is_kept() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Simple("OK".to_owned()));
    let client = connected(&server).await;
    let reply = client.lpop("list", None).await.expect("a reply");
    assert_eq!(reply, Value::SimpleString("OK".to_owned()));
}
