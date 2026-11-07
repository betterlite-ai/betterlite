use std::time::{Duration, Instant};

use tokio::time::sleep;

use redis::cmd;
use rstest::rstest;
use secrecy::SecretString;

use super::fake_server::FakeServer;
use super::fixtures::{direct_client, direct_fields, empty_fields, factory, open_error};
use crate::redis_client_factories::RedisClient;
use crate::redis_live_client::connection_info;
use crate::redis_live_client::{LiveClientError, LiveTarget};

#[tokio::test]
async fn direct_connect_carries_host_port_db_username_and_password() {
    let server = FakeServer::start().await;
    let mut fields = direct_fields(&server.host(), server.port());
    fields.db = Some(2);
    fields.username = Some("app".to_owned());
    fields.password = Some(SecretString::from("pw".to_owned()));
    let client = factory(None)
        .open_async(&RedisClient::Direct(fields), None)
        .await
        .expect("a direct connection");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
    let auth = server.commands_named("AUTH");
    assert_eq!(auth.len(), 1);
    assert_eq!(auth[0].args, ["AUTH", "app", "pw"]);
    let select = server.commands_named("SELECT");
    assert_eq!(select.len(), 1);
    assert_eq!(select[0].args, ["SELECT", "2"]);
}

#[tokio::test]
async fn url_connect_carries_db_credentials_and_the_record_username() {
    let server = FakeServer::start().await;
    let url = format!(
        "redis://url-user:url-password@{}:{}/3",
        server.host(),
        server.port()
    );
    let mut record = super::fixtures::url_client(url);
    record.username = Some("record-user".to_owned());
    let client = factory(None)
        .open_async(&RedisClient::Url(record), None)
        .await
        .expect("a url connection");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
    let auth = server.commands_named("AUTH");
    assert_eq!(auth[0].args, ["AUTH", "record-user", "url-password"]);
    let select = server.commands_named("SELECT");
    assert_eq!(select[0].args, ["SELECT", "3"]);
}

#[test]
fn defaults_are_localhost_6379_and_db_zero() {
    let endpoint = factory(None)
        .blocking_endpoint(&RedisClient::Direct(empty_fields()))
        .expect("an endpoint");
    let LiveTarget::Standalone(client) = endpoint.target else {
        panic!("a direct record builds a standalone target");
    };
    let info = client.get_connection_info();
    assert_eq!(info.addr().to_string(), "localhost:6379");
    assert_eq!(info.redis_settings().db(), 0);
    assert_eq!(info.redis_settings().username(), None);
}

#[test]
fn no_socket_timeout_uses_five_seconds() {
    let defaults = connection_info::timeouts(None, None).expect("the default timeouts");
    assert_eq!(defaults.response, Duration::from_secs(5));
    assert_eq!(defaults.connect, Duration::from_secs(5));
    let response_only = connection_info::timeouts(Some(0.5_f64), None).expect("a response timeout");
    assert_eq!(response_only.response, Duration::from_millis(500));
    assert_eq!(response_only.connect, Duration::from_millis(500));
    let both = connection_info::timeouts(Some(0.5_f64), Some(0.25_f64)).expect("two timeouts");
    assert_eq!(both.response, Duration::from_millis(500));
    assert_eq!(both.connect, Duration::from_millis(250));
}

#[rstest]
#[case::negative(-1.0_f64)]
#[case::not_a_number(f64::NAN)]
#[tokio::test]
async fn an_invalid_timeout_is_an_invalid_setting(#[case] seconds: f64) {
    let mut fields = direct_fields("127.0.0.1", 6379);
    fields.socket_timeout = Some(seconds);
    let error = open_error(&factory(None), &RedisClient::Direct(fields)).await;
    assert!(
        matches!(error, LiveClientError::InvalidSetting { field } if field == "socket_timeout")
    );
}

#[tokio::test]
async fn a_port_above_65535_is_an_invalid_setting() {
    let mut fields = direct_fields("127.0.0.1", 6379);
    fields.port = Some(70_000);
    let error = open_error(&factory(None), &RedisClient::Direct(fields)).await;
    assert!(matches!(error, LiveClientError::InvalidSetting { field } if field == "port"));
}

#[tokio::test]
async fn a_server_that_never_answers_fails_at_the_response_timeout() {
    let server = FakeServer::start().await;
    server.silence("PING");
    let mut fields = direct_fields(&server.host(), server.port());
    fields.socket_timeout = Some(0.2_f64);
    let client = factory(None)
        .open_async(&RedisClient::Direct(fields), None)
        .await
        .expect("a direct connection");
    let started = Instant::now();
    let error = client
        .query::<String>(&cmd("PING"))
        .await
        .expect_err("a timeout");
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(150), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    assert!(matches!(error, LiveClientError::Redis(_)));
}

#[tokio::test]
async fn a_dropped_connection_reconnects_on_the_next_command() {
    let server = FakeServer::start().await;
    server.close_after("PING");
    let client = factory(None)
        .open_async(&direct_client(&server), None)
        .await
        .expect("a direct connection");
    let first: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(first, "PONG");
    let _lost = client.query::<String>(&cmd("PING")).await;
    let reconnected: String = client
        .query(&cmd("PING"))
        .await
        .expect("the reconnected manager answers");
    assert_eq!(reconnected, "PONG");
    assert!(server.connections() >= 2, "a new connection is opened");
}

#[tokio::test]
async fn max_connections_reaches_the_concurrency_limit() {
    let server = FakeServer::start().await;
    server.silence("PING");
    let mut fields = direct_fields(&server.host(), server.port());
    fields.socket_timeout = Some(0.3_f64);
    fields.max_connections = Some(1);
    let client = factory(None)
        .open_async(&RedisClient::Direct(fields), None)
        .await
        .expect("a direct connection");
    let command = cmd("PING");
    let queries = async {
        tokio::join!(
            client.query::<String>(&command),
            client.query::<String>(&command),
        )
    };
    let observing = async {
        sleep(Duration::from_millis(100)).await;
        assert_eq!(
            server.commands_named("PING").len(),
            1,
            "the concurrency limit holds the second command"
        );
    };
    let ((first, second), ()) = tokio::join!(queries, observing);
    assert!(first.is_err() && second.is_err(), "both commands time out");
    assert_eq!(server.commands_named("PING").len(), 2);
}

#[test]
fn max_connections_prefers_the_pool_then_the_record_then_1024() {
    assert_eq!(
        connection_info::concurrency_limit(None, None).expect("the default"),
        1024
    );
    assert_eq!(
        connection_info::concurrency_limit(Some(7), None).expect("the record limit"),
        7
    );
    assert_eq!(
        connection_info::concurrency_limit(Some(7), Some(9)).expect("the pool limit"),
        9
    );
    let error = connection_info::concurrency_limit(Some(-1), None).expect_err("a negative limit");
    assert!(
        matches!(error, LiveClientError::InvalidSetting { field } if field == "max_connections")
    );
}
