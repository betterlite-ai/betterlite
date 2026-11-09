use super::empty_settings;
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::settings::RedisConnectionSettings;
use crate::redis_live_client::tests::fake_server::FakeServer;

pub(crate) fn cluster_settings(nodes: &[(String, u16)]) -> RedisConnectionSettings {
    let mut settings = empty_settings();
    settings.startup_nodes = Some(
        nodes
            .iter()
            .map(|(host, port)| ClusterNodeConfig {
                host: host.clone(),
                port: i64::from(*port),
            })
            .collect(),
    );
    settings
}

pub(crate) async fn cluster_pair() -> (FakeServer, FakeServer) {
    let first = FakeServer::start().await;
    let second = FakeServer::start().await;
    let slots = vec![
        (0, 8191, vec![(first.host(), first.port())]),
        (8192, 16383, vec![(second.host(), second.port())]),
    ];
    first.set_cluster_slots(slots.clone());
    second.set_cluster_slots(slots);
    (first, second)
}
