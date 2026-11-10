use redis::aio::MultiplexedConnection;
use redis::{Client, RedisResult, Value, cmd, from_redis_value_ref};

use super::support::{TestClock, breaker, route_key, store};
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_value_store::{PinValue, RouteIncrement, RouteValue, StoreRead};

pub(crate) async fn run(name: &str) {
    match name {
        "fake_server_serves_commands" => serves_commands().await,
        "fake_server_reports_error_replies" => reports_error_replies().await,
        "mismatched_compare_releases_its_watch" => mismatched_compare_releases_its_watch().await,
        "rival_created_key_gets_the_default_expiry" => {
            rival_created_key_gets_the_default_expiry().await;
        }
        other => super::classes::run(other),
    }
}

async fn open(server: &FakeRedis) -> MultiplexedConnection {
    Client::open(format!("redis://127.0.0.1:{}", server.port()))
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

async fn send(connection: &mut MultiplexedConnection, words: &[&str]) -> RedisResult<Value> {
    let (name, rest) = words.split_first().unwrap();
    let mut command = cmd(name);
    for word in rest {
        command.arg(*word);
    }
    command.query_async(connection).await
}

fn show(value: &Value) -> String {
    if matches!(value, Value::Nil) {
        return "nil".to_owned();
    }
    if let Value::Array(items) = value {
        return items.iter().map(show).collect::<Vec<_>>().join(",");
    }
    from_redis_value_ref::<String>(value).unwrap()
}

async fn run_script(server: &FakeRedis, script: &[(&[&str], &str)]) {
    let mut connection = open(server).await;
    for (words, expected) in script {
        let reply = send(&mut connection, words).await.unwrap();
        assert_eq!(show(&reply), *expected, "{words:?}");
    }
}

async fn serves_commands() {
    let server = FakeRedis::start().await;
    let script: [(&[&str], &str); 26] = [
        (&["PING"], "PONG"),
        (&["SET", "k", "v", "EX", "100"], "OK"),
        (&["TTL", "k"], "100"),
        (&["SET", "k", "w", "KEEPTTL"], "OK"),
        (&["TTL", "k"], "100"),
        (&["GET", "k"], "w"),
        (&["SET", "fresh", "v", "KEEPTTL"], "OK"),
        (&["TTL", "fresh"], "-1"),
        (&["SET", "plain", "v"], "OK"),
        (&["TTL", "plain"], "-1"),
        (&["SET", "plain", "x", "NX"], "nil"),
        (&["SET", "new", "x", "NX"], "OK"),
        (&["TTL", "absent"], "-2"),
        (&["MGET", "k", "absent", "new"], "w,nil,x"),
        (&["EXPIRE", "k", "7"], "1"),
        (&["EXPIRE", "absent", "7"], "0"),
        (&["TTL", "k"], "7"),
        (&["SET", "a", "1"], "OK"),
        (&["SET", "b", "2"], "OK"),
        (&["DEL", "a", "b", "absent"], "2"),
        (&["DEL", "a"], "0"),
        (&["GET", "a"], "nil"),
        (&["INCRBYFLOAT", "n", "1.5"], "1.5"),
        (&["INCRBYFLOAT", "n", "2"], "3.5"),
        (&["GET", "n"], "3.5"),
        (&["TTL", "n"], "-1"),
    ];
    run_script(&server, &script).await;
}

async fn reports_error_replies() {
    let server = FakeRedis::start().await;
    server.reject_auth(true);
    let mut connection = open(&server).await;
    let cases: [(&[&str], &str, &str); 3] = [
        (
            &["AUTH", "wrong"],
            "WRONGPASS",
            "invalid username-password pair",
        ),
        (&["EXEC"], "ERR", "EXEC without MULTI"),
        (
            &["SET", "k", "v", "EX", "0"],
            "ERR",
            "invalid expire time in 'set' command",
        ),
    ];
    for (words, code, detail) in cases {
        let error = send(&mut connection, words).await.unwrap_err();
        assert_eq!(error.code(), Some(code), "{words:?}");
        assert!(
            error.detail().is_some_and(|text| text.contains(detail)),
            "{words:?}: {error:?}"
        );
    }
}

async fn mismatched_compare_releases_its_watch() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    let held = route_key("held");
    let free = route_key("free");
    let first = RouteValue::Pin(PinValue {
        model_id: "first".to_owned(),
    });
    let second = RouteValue::Pin(PinValue {
        model_id: "second".to_owned(),
    });
    store
        .compare_and_set(&held, &StoreRead::Missing, &first, None)
        .await
        .unwrap();
    let mismatch = store
        .compare_and_set(&held, &StoreRead::Missing, &second, None)
        .await
        .unwrap();
    assert!(!mismatch.value.swapped);
    assert_eq!(mismatch.value.current, StoreRead::Value(first));
    server.seed("held", b"{}", -1);
    let claimed = store
        .compare_and_set(&free, &StoreRead::Missing, &second, None)
        .await
        .unwrap();
    assert!(claimed.value.swapped);
}

async fn rival_created_key_gets_the_default_expiry() {
    let server = FakeRedis::start().await;
    let clock = TestClock::new();
    let store = store(&server, None, breaker(5, 60_000), &clock);
    server.rival_writes_before_exec(1);
    let observed = store
        .increment(
            &route_key("counter"),
            &RouteIncrement::Counter(1),
            Some(100.0_f64),
        )
        .await
        .unwrap();
    assert_eq!(observed.value, RouteValue::Counter(2));
    let (_bytes, ttl) = server.raw("counter").unwrap();
    assert_eq!(ttl, 100);
}
