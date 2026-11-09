use super::support::cluster::{cluster_pair, cluster_settings};
use super::support::{Probe, direct_settings, probe, script_after_connection_setup};
use crate::redis_cache_admin::info_parse::InfoValue;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

const CLIENT_LIST: &str = "id=3 addr=127.0.0.1:56789 name= age=5 cmd=client|list\nid=4 addr=127.0.0.1:56790 name= age=2 cmd=ping\n";
const INFO_TEXT: &str = "# Server\r\nredis_version:7.2.4\r\nuptime_in_seconds:42\r\n";

fn standalone(server: &FakeServer) -> Probe {
    probe(
        &direct_settings(&server.host(), server.port()),
        RedisCacheFamily::Standalone,
        None,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_list_parses_the_sync_reply() {
    let server = FakeServer::start().await;
    script_after_connection_setup(&server, "CLIENT", Reply::Bulk(CLIENT_LIST.to_owned()));
    let fixture = standalone(&server);
    let clients = fixture.admin.client_list().expect("a client list");
    assert_eq!(clients.len(), 2);
    assert_eq!(
        clients[0].get("addr").map(String::as_str),
        Some("127.0.0.1:56789")
    );
    assert_eq!(clients[1].get("cmd").map(String::as_str), Some("ping"));
    let lists = server
        .commands_named("CLIENT")
        .into_iter()
        .filter(|command| command.args.get(1).map(String::as_str) == Some("LIST"))
        .count();
    assert_eq!(lists, 1);
    assert!(fixture.gate.enters().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_list_returns_the_server_error() {
    let server = FakeServer::start().await;
    script_after_connection_setup(
        &server,
        "CLIENT",
        Reply::Error("ERR client list is restricted".to_owned()),
    );
    let fixture = standalone(&server);
    let error = fixture
        .admin
        .client_list()
        .expect_err("the scripted client list fails");
    assert!(error.to_string().contains("client list is restricted"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn info_parses_the_sync_reply() {
    let server = FakeServer::start().await;
    server.script("INFO", Reply::Bulk(INFO_TEXT.to_owned()));
    let fixture = standalone(&server);
    let info = fixture.admin.info().expect("an info reply");
    assert_eq!(
        info.get("redis_version"),
        Some(&InfoValue::Text("7.2.4".to_owned()))
    );
    assert_eq!(info.get("uptime_in_seconds"), Some(&InfoValue::Int(42)));
    assert!(!info.contains_key("# Server"));
    assert_eq!(server.commands_named("INFO").len(), 1);
    assert!(fixture.gate.enters().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flush_cache_and_flushall_each_send_flushall() {
    let server = FakeServer::start().await;
    let fixture = standalone(&server);
    fixture.admin.flush_cache().expect("a flush");
    fixture.admin.flushall().expect("a flush");
    assert_eq!(server.commands_named("FLUSHALL").len(), 2);
    assert!(fixture.gate.enters().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn info_takes_one_node_reply_from_a_cluster_client() {
    let (first, second) = cluster_pair().await;
    first.script(
        "INFO",
        Reply::Bulk("# Server\r\nredis_version:7.2.4\r\n".to_owned()),
    );
    second.script(
        "INFO",
        Reply::Bulk("# Server\r\nredis_version:7.4.0\r\n".to_owned()),
    );
    let settings =
        cluster_settings(&[(first.host(), first.port()), (second.host(), second.port())]);
    let fixture = probe(&settings, RedisCacheFamily::Cluster, None);
    let info = fixture.admin.info().expect("a cluster info reply");
    let version = match info.get("redis_version") {
        Some(InfoValue::Text(text)) => text.clone(),
        other => panic!("a redis version text, got {other:?}"),
    };
    assert!(
        version == "7.2.4" || version == "7.4.0",
        "the cluster info comes from one node, got {version}"
    );
    assert_eq!(first.commands_named("INFO").len(), 1);
    assert_eq!(second.commands_named("INFO").len(), 1);
}
