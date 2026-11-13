use std::time::{Duration, Instant};

use redis::Value;
use redis::cmd;
use secrecy::SecretString;

use super::fake_server::{FakeServer, Reply};
use super::fixtures::{closed_address, credential, factory, open_error, sentinel_record, source};
use crate::redis_client_factories::RedisClient;
use crate::redis_live_client::LiveClientError;

fn monitor_for(monitor: &FakeServer, master: &FakeServer) {
    monitor.set_sentinel_master(master.host(), master.port());
}

#[tokio::test]
async fn a_dead_monitor_is_skipped() {
    let master = FakeServer::start().await;
    let monitor = FakeServer::start().await;
    monitor_for(&monitor, &master);
    let record = sentinel_record(&[closed_address(), monitor.addr()], "mymaster");
    let client = factory(None)
        .open_async(&RedisClient::Sentinel(record), None)
        .await
        .expect("a sentinel connection");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
    assert_eq!(monitor.commands_named("SENTINEL").len(), 1);
    assert_eq!(master.commands_named("PING").len(), 1);
}

#[tokio::test]
async fn a_nil_answer_moves_to_the_next_monitor() {
    let master = FakeServer::start().await;
    let first = FakeServer::start().await;
    let second = FakeServer::start().await;
    first.clear_sentinel_master();
    monitor_for(&second, &master);
    let record = sentinel_record(&[first.addr(), second.addr()], "mymaster");
    let client = factory(None)
        .open_async(&RedisClient::Sentinel(record), None)
        .await
        .expect("a sentinel connection");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
    assert_eq!(first.commands_named("SENTINEL").len(), 1);
    assert_eq!(second.commands_named("SENTINEL").len(), 1);
}

#[tokio::test]
async fn no_monitor_knowing_the_service_is_an_error() {
    let first = FakeServer::start().await;
    let second = FakeServer::start().await;
    first.clear_sentinel_master();
    second.clear_sentinel_master();
    let record = sentinel_record(&[first.addr(), second.addr()], "missing-service");
    let error = open_error(&factory(None), &RedisClient::Sentinel(record)).await;
    assert!(
        matches!(error, LiveClientError::NoSentinelMaster { service_name } if service_name == "missing-service")
    );
}

#[tokio::test]
async fn monitors_get_the_monitor_password_and_never_the_data_token() {
    let master = FakeServer::start().await;
    let monitor = FakeServer::start().await;
    monitor_for(&monitor, &master);
    let mut record = sentinel_record(&[monitor.addr()], "mymaster");
    record.monitor_password = Some(SecretString::from("monitor-secret".to_owned()));
    record.connection_kwargs.username = Some("monitor-user".to_owned());
    record.connection_kwargs.credential_provider = true;
    let credential_source = source(vec![credential(Some("data-user"), "data-token")]);
    let client = factory(Some(credential_source))
        .open_async(&RedisClient::Sentinel(record), None)
        .await
        .expect("a sentinel connection");
    client.query::<String>(&cmd("PING")).await.expect("a reply");
    let monitor_auth = monitor.commands_named("AUTH");
    assert_eq!(
        monitor_auth[0].args,
        ["AUTH", "monitor-user", "monitor-secret"]
    );
    assert!(
        !monitor_auth
            .iter()
            .any(|recorded| recorded.args.iter().any(|arg| arg == "data-token")),
        "the data node token never reaches a monitor"
    );
    let master_auth = master.commands_named("AUTH");
    assert!(
        master_auth
            .iter()
            .any(|recorded| recorded.args == ["AUTH", "data-user", "data-token"]),
        "the data node authenticates with the injected token"
    );
}

#[tokio::test]
async fn a_dropped_connection_resolves_the_master_again() {
    let first_master = FakeServer::start().await;
    first_master.close_after("PING");
    let second_master = FakeServer::start().await;
    let monitor = FakeServer::start().await;
    monitor_for(&monitor, &first_master);
    let record = sentinel_record(&[monitor.addr()], "mymaster");
    let client = factory(None)
        .open_async(&RedisClient::Sentinel(record), None)
        .await
        .expect("a sentinel connection");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
    let lost = client.query::<String>(&cmd("PING")).await;
    assert!(lost.is_err(), "the closed master fails the command");
    monitor_for(&monitor, &second_master);
    let recovered: String = client
        .query(&cmd("PING"))
        .await
        .expect("the new master answers");
    assert_eq!(recovered, "PONG");
    assert_eq!(
        monitor.commands_named("SENTINEL").len(),
        2,
        "the monitors were asked again"
    );
    assert_eq!(second_master.commands_named("PING").len(), 1);
}

#[tokio::test]
async fn a_readonly_reply_resolves_the_master_again() {
    let first_master = FakeServer::start().await;
    first_master.script(
        "SET",
        Reply::Error("READONLY You can't write against a read only replica.".to_owned()),
    );
    let second_master = FakeServer::start().await;
    let monitor = FakeServer::start().await;
    monitor_for(&monitor, &first_master);
    let record = sentinel_record(&[monitor.addr()], "mymaster");
    let client = factory(None)
        .open_async(&RedisClient::Sentinel(record), None)
        .await
        .expect("a sentinel connection");
    let rejected = client
        .query::<Value>(cmd("SET").arg("key").arg("value"))
        .await;
    assert!(
        matches!(rejected, Err(LiveClientError::Redis(error)) if error.code() == Some("READONLY"))
    );
    monitor_for(&monitor, &second_master);
    let accepted: Value = client
        .query(cmd("SET").arg("key").arg("value"))
        .await
        .expect("the new master accepts writes");
    assert_eq!(accepted, Value::Okay);
    assert_eq!(monitor.commands_named("SENTINEL").len(), 2);
}

#[tokio::test]
async fn a_mute_monitor_is_bounded_by_the_timeouts() {
    let master = FakeServer::start().await;
    let mute = FakeServer::start().await;
    mute.mute();
    let monitor = FakeServer::start().await;
    monitor_for(&monitor, &master);
    let mut record = sentinel_record(&[mute.addr(), monitor.addr()], "mymaster");
    record.connection_kwargs.socket_timeout = Some(0.1_f64);
    let started = Instant::now();
    let client = factory(None)
        .open_async(&RedisClient::Sentinel(record), None)
        .await
        .expect("the second monitor answers");
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(80), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    assert!(mute.connections() >= 1, "the mute monitor was tried");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
}
