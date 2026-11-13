use rstest::rstest;

use super::support::{Hooks, Logs, Probe, ScriptedPing, closed_port, direct_settings, probe};
use crate::redis_cache_admin::health::ping_commands;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn standalone(server: &FakeServer) -> Probe {
    probe(
        &direct_settings(&server.host(), server.port()),
        RedisCacheFamily::Standalone,
        None,
    )
}

#[tokio::test]
async fn ping_records_a_success_with_the_caller() {
    let server = FakeServer::start().await;
    let fixture = standalone(&server);
    let answer = fixture
        .admin
        .ping(Some("cache_route"))
        .await
        .expect("a ping");
    assert!(answer);
    let successes = fixture.hooks.successes();
    assert_eq!(successes.len(), 1);
    assert_eq!(successes[0].call_type, "async_ping");
    assert_eq!(successes[0].key.as_deref(), Some("cache_route"));
    assert!(fixture.hooks.failures().is_empty());
    assert_eq!(server.commands_named("PING").len(), 1);
}

#[derive(Clone, Copy, Debug)]
enum FailureSource {
    Scripted,
    Refused,
}

#[rstest]
#[case::scripted_ping_error(FailureSource::Scripted)]
#[case::refused_connection(FailureSource::Refused)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ping_records_a_failure_and_logs_the_ping_text(#[case] source: FailureSource) {
    let server = match source {
        FailureSource::Scripted => Some(FakeServer::start().await),
        FailureSource::Refused => None,
    };
    if let Some(running) = &server {
        running.script("PING", Reply::Error("ERR redis is down".to_owned()));
    }
    let (logs, _guard) = Logs::capture();
    let settings = server.as_ref().map_or_else(
        || direct_settings("127.0.0.1", closed_port()),
        |running| direct_settings(&running.host(), running.port()),
    );
    let fixture = probe(&settings, RedisCacheFamily::Standalone, None);
    let error = fixture
        .admin
        .ping(Some("cache_route"))
        .await
        .expect_err("the ping fails");
    let text = error.to_string();
    let failures = fixture.hooks.failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].event.call_type, "async_ping");
    assert_eq!(failures[0].event.key.as_deref(), Some("cache_route"));
    assert_eq!(failures[0].error, text);
    let lines = logs.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("LiteLLM Redis Cache PING: - Got exception from REDIS : "))
    );
    assert!(lines.iter().any(|line| line.contains(text.as_str())));
}

#[tokio::test]
async fn a_false_ping_is_a_success() {
    let hooks = Hooks::default();
    let client = ScriptedPing::answering(false);
    let answer = ping_commands(&client, &hooks, Some("cache_route"))
        .await
        .expect("a false ping is answered");
    assert!(!answer);
    assert_eq!(client.calls(), 1);
    let successes = hooks.successes();
    assert_eq!(successes.len(), 1);
    assert_eq!(successes[0].call_type, "async_ping");
    assert_eq!(successes[0].key.as_deref(), Some("cache_route"));
    assert!(hooks.failures().is_empty());
}

#[tokio::test]
async fn the_ping_seam_records_a_failure_with_its_error() {
    let hooks = Hooks::default();
    let client = ScriptedPing::failing("the scripted connection dropped");
    let error = ping_commands(&client, &hooks, None)
        .await
        .expect_err("the scripted ping fails");
    assert_eq!(error.to_string(), "the scripted connection dropped");
    let failures = hooks.failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].event.call_type, "async_ping");
    assert_eq!(failures[0].event.key, None);
    assert_eq!(failures[0].error, "the scripted connection dropped");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_ping_returns_true_for_a_pong_reply() {
    let server = FakeServer::start().await;
    let fixture = standalone(&server);
    let answer = fixture
        .admin
        .sync_ping(Some("startup"))
        .expect("a sync ping");
    assert!(answer);
    let successes = fixture.hooks.successes();
    assert_eq!(successes.len(), 1);
    assert_eq!(successes[0].call_type, "sync_ping");
    assert_eq!(successes[0].key.as_deref(), Some("startup"));
    assert_eq!(server.commands_named("PING").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_ping_answers_false_for_another_reply_and_still_records_a_success() {
    let server = FakeServer::start().await;
    server.script("PING", Reply::Simple("PONGISH".to_owned()));
    let fixture = standalone(&server);
    let answer = fixture.admin.sync_ping(None).expect("a sync ping");
    assert!(!answer);
    let successes = fixture.hooks.successes();
    assert_eq!(successes.len(), 1);
    assert_eq!(successes[0].call_type, "sync_ping");
    assert_eq!(successes[0].key, None);
    assert!(fixture.hooks.failures().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_ping_records_a_failure_and_logs_the_ping_text() {
    let server = FakeServer::start().await;
    server.script("PING", Reply::Error("ERR redis is down".to_owned()));
    let (logs, _guard) = Logs::capture();
    let fixture = standalone(&server);
    let error = fixture
        .admin
        .sync_ping(Some("startup"))
        .expect_err("the scripted ping fails");
    assert!(error.to_string().contains("redis is down"));
    let failures = fixture.hooks.failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].event.call_type, "sync_ping");
    assert_eq!(failures[0].event.key.as_deref(), Some("startup"));
    let lines = logs.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("LiteLLM Redis Cache PING: - Got exception from REDIS : "))
    );
    assert!(lines.iter().any(|line| line.contains("redis is down")));
}
