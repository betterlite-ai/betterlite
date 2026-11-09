use std::time::Duration;

use tokio::time::sleep;

use super::support::{Hooks, Probe, direct_settings, probe, wait_for_commands};
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn standalone(server: &FakeServer) -> Probe {
    probe(
        &direct_settings(&server.host(), server.port()),
        RedisCacheFamily::Standalone,
        None,
    )
}

async fn wait_for_successes(hooks: &Hooks, count: usize) {
    for _attempt in 0_usize..200 {
        if hooks.successes().len() >= count {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("the health ping events did not arrive");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn setup_health_pings_sends_one_async_and_one_sync_ping() {
    let server = FakeServer::start().await;
    let fixture = standalone(&server);
    fixture.admin.setup_health_pings().await;
    wait_for_commands(&server, "PING", 2).await;
    wait_for_successes(&fixture.hooks, 1).await;
    assert_eq!(server.commands_named("PING").len(), 2);
    let successes = fixture.hooks.successes();
    assert_eq!(successes.len(), 1);
    assert_eq!(successes[0].call_type, "async_ping");
    assert_eq!(successes[0].key, None);
    assert!(fixture.hooks.failures().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_sync_probe_records_one_redis_sync_ping_failure() {
    let server = FakeServer::start().await;
    server.script("PING", Reply::Error("ERR redis is down".to_owned()));
    server.script("PING", Reply::Error("ERR redis is down".to_owned()));
    let fixture = standalone(&server);
    fixture.admin.setup_health_pings().await;
    let sync_failures: Vec<String> = fixture
        .hooks
        .failures()
        .into_iter()
        .filter(|failure| failure.event.call_type == "redis_sync_ping")
        .map(|failure| failure.error)
        .collect();
    assert_eq!(sync_failures.len(), 1);
    assert!(sync_failures[0].contains("redis is down"));
    let event = fixture
        .hooks
        .failures()
        .into_iter()
        .find(|failure| failure.event.call_type == "redis_sync_ping")
        .expect("the sync ping failure event");
    assert_eq!(event.event.duration, Duration::ZERO);
    assert_eq!(event.event.key, None);
}
