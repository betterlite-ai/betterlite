use super::support::cluster::{cluster_pair, cluster_settings};
use super::support::{Probe, direct_settings, probe};
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_live_client::tests::fake_server::FakeServer;

fn standalone(server: &FakeServer) -> Probe {
    probe(
        &direct_settings(&server.host(), server.port()),
        RedisCacheFamily::Standalone,
        None,
    )
}

#[tokio::test]
async fn disconnect_drops_the_standalone_async_client() {
    let server = FakeServer::start().await;
    let fixture = standalone(&server);
    assert!(fixture.admin.ping(None).await.expect("a first ping"));
    assert_eq!(server.connections(), 1);
    fixture.admin.disconnect().await;
    assert!(fixture.admin.ping(None).await.expect("a second ping"));
    assert_eq!(
        server.connections(),
        2,
        "the cached client was dropped and a new one opened"
    );
}

#[tokio::test]
async fn disconnect_can_be_called_twice() {
    let server = FakeServer::start().await;
    let fixture = standalone(&server);
    fixture.admin.disconnect().await;
    fixture.admin.disconnect().await;
    assert!(fixture.hooks.successes().is_empty());
    assert!(fixture.hooks.failures().is_empty());
}

#[tokio::test]
async fn disconnect_drops_the_shared_pubsub_client_on_a_cluster() {
    let (first, second) = cluster_pair().await;
    let settings =
        cluster_settings(&[(first.host(), first.port()), (second.host(), second.port())]);
    let fixture = probe(&settings, RedisCacheFamily::Cluster, None);
    drop(
        fixture
            .admin
            .transport
            .pubsub_client()
            .await
            .expect("the pubsub client"),
    );
    let before = first.connections() + second.connections();
    drop(
        fixture
            .admin
            .transport
            .pubsub_client()
            .await
            .expect("the shared pubsub client"),
    );
    assert_eq!(first.connections() + second.connections(), before);
    fixture.admin.disconnect().await;
    drop(
        fixture
            .admin
            .transport
            .pubsub_client()
            .await
            .expect("the new pubsub client"),
    );
    assert!(
        first.connections() + second.connections() > before,
        "disconnect removed the shared pubsub client"
    );
}
