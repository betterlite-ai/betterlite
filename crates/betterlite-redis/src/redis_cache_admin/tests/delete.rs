use super::support::cluster::{cluster_pair, cluster_settings};
use super::support::{Probe, direct_settings, probe};
use crate::redis_cache_admin::RedisAdminError;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn standalone(server: &FakeServer, namespace: Option<&str>) -> Probe {
    probe(
        &direct_settings(&server.host(), server.port()),
        RedisCacheFamily::Standalone,
        namespace,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_cache_keys_namespaces_the_keys_and_sends_one_del() {
    let server = FakeServer::start().await;
    server.script("DEL", Reply::Int(2));
    let fixture = standalone(&server, Some("cache"));
    fixture
        .admin
        .delete_cache_keys(&["alpha".to_owned(), "beta".to_owned()])
        .await
        .expect("a delete");
    let deletes = server.commands_named("DEL");
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].args, ["DEL", "cache:alpha", "cache:beta"]);
    assert_eq!(fixture.gate.enters(), ["delete_cache_keys"]);
    assert_eq!(fixture.gate.exits(), [11]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_cache_keys_sends_nothing_when_the_breaker_is_open() {
    let server = FakeServer::start().await;
    let fixture = standalone(&server, None);
    fixture.gate.open_for("delete_cache_keys");
    let error = fixture
        .admin
        .delete_cache_keys(&["alpha".to_owned()])
        .await
        .expect_err("the breaker is open");
    assert!(matches!(error, RedisAdminError::Breaker(_)));
    assert!(server.commands_named("DEL").is_empty());
    assert_eq!(fixture.gate.enters(), ["delete_cache_keys"]);
    assert!(fixture.gate.exits().is_empty());
    assert!(fixture.gate.failures().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_cache_keys_reports_its_error_to_the_gate() {
    let server = FakeServer::start().await;
    server.script("DEL", Reply::Error("ERR no delete for you".to_owned()));
    let fixture = standalone(&server, None);
    let error = fixture
        .admin
        .delete_cache_keys(&["alpha".to_owned()])
        .await
        .expect_err("the scripted delete fails");
    assert!(error.to_string().contains("no delete for you"));
    assert!(fixture.gate.exits().is_empty());
    let failures = fixture.gate.failures();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("no delete for you"));
}

#[tokio::test]
async fn async_delete_cache_namespaces_the_key_and_returns_the_count() {
    let server = FakeServer::start().await;
    server.script("DEL", Reply::Int(2));
    let fixture = standalone(&server, Some("cache"));
    let count = fixture
        .admin
        .async_delete_cache("alpha")
        .await
        .expect("a delete");
    assert_eq!(count, 2);
    let deletes = server.commands_named("DEL");
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].args, ["DEL", "cache:alpha"]);
    assert_eq!(fixture.gate.enters(), ["async_delete_cache"]);
    assert_eq!(fixture.gate.exits(), [11]);
}

#[tokio::test]
async fn async_delete_cache_sends_nothing_when_the_breaker_is_open() {
    let server = FakeServer::start().await;
    let fixture = standalone(&server, None);
    fixture.gate.open_for("async_delete_cache");
    let error = fixture
        .admin
        .async_delete_cache("alpha")
        .await
        .expect_err("the breaker is open");
    assert!(matches!(error, RedisAdminError::Breaker(_)));
    assert!(server.commands_named("DEL").is_empty());
    assert!(fixture.gate.exits().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_cache_namespaces_the_key_and_skips_the_gate() {
    let server = FakeServer::start().await;
    server.script("DEL", Reply::Int(1));
    let fixture = standalone(&server, Some("cache"));
    fixture.admin.delete_cache("alpha").expect("a delete");
    let deletes = server.commands_named("DEL");
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].args, ["DEL", "cache:alpha"]);
    assert!(fixture.gate.enters().is_empty());
    assert!(fixture.gate.exits().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_key_list_sends_nothing_on_a_cluster_client() {
    let (first, second) = cluster_pair().await;
    let settings =
        cluster_settings(&[(first.host(), first.port()), (second.host(), second.port())]);
    let fixture = probe(&settings, RedisCacheFamily::Cluster, None);
    fixture
        .admin
        .delete_cache_keys(&[])
        .await
        .expect("an empty delete");
    assert!(first.commands_named("DEL").is_empty());
    assert!(second.commands_named("DEL").is_empty());
    assert_eq!(fixture.gate.exits(), [11]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_key_list_sends_a_bare_del_on_a_standalone_client() {
    let server = FakeServer::start().await;
    server.script(
        "DEL",
        Reply::Error("ERR wrong number of arguments for 'del' command".to_owned()),
    );
    let fixture = standalone(&server, None);
    let error = fixture
        .admin
        .delete_cache_keys(&[])
        .await
        .expect_err("a bare DEL is a server error");
    assert!(error.to_string().contains("wrong number of arguments"));
    let deletes = server.commands_named("DEL");
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].args, ["DEL"]);
}
