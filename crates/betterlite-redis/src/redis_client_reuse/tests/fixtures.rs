use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::redis_client_factories::RedisClient;
use crate::redis_client_factories::cluster::ClusterClient;
use crate::redis_client_factories::dispatch::build_client;
use crate::redis_client_factories::pool::PoolConfig;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::settings::{ClientKind, RedisConnectionSettings};
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::credentials::{
    RedisCredentialError, RedisCredentialSource, RedisCredentials,
};
use crate::redis_live_client::tests::fake_server::FakeServer;
use secrecy::SecretString;

pub(crate) mod pubsub_server;

pub(crate) fn empty_settings() -> RedisConnectionSettings {
    RedisConnectionSettings {
        host: None,
        port: None,
        db: None,
        username: None,
        password: None,
        url: None,
        ssl: None,
        socket_timeout: None,
        socket_connect_timeout: None,
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        credential_provider_settings: None,
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        passthrough: BTreeMap::new(),
    }
}

pub(crate) fn direct_settings(host: &str, port: u16) -> RedisConnectionSettings {
    let mut settings = empty_settings();
    settings.host = Some(host.to_owned());
    settings.port = Some(i64::from(port));
    settings
}

pub(crate) fn cluster_settings(nodes: &[(String, u16)]) -> RedisConnectionSettings {
    let mut settings = empty_settings();
    settings.startup_nodes = Some(nodes.iter().map(node_config).collect());
    settings
}

pub(crate) fn cluster_record(nodes: &[(String, u16)]) -> ClusterClient {
    ClusterClient {
        startup_nodes: nodes.iter().map(node_config).collect(),
        username: None,
        password: None,
        ssl: None,
        ssl_password: None,
        socket_timeout: None,
        socket_connect_timeout: None,
        max_connections: None,
        health_check_interval: None,
        socket_keepalive: None,
        credential_provider: false,
        credential_provider_settings: None,
        passthrough: BTreeMap::new(),
        isolation: None,
    }
}

fn node_config(node: &(String, u16)) -> ClusterNodeConfig {
    ClusterNodeConfig {
        host: node.0.clone(),
        port: i64::from(node.1),
    }
}

pub(crate) fn client(settings: &RedisConnectionSettings, kind: ClientKind) -> RedisClient {
    build_client(settings, kind).expect("a redis client record")
}

pub(crate) fn factory() -> LiveClientFactory {
    LiveClientFactory::new(None)
}

pub(crate) fn store() -> Arc<RedisClientStore> {
    Arc::new(RedisClientStore::new())
}

pub(crate) fn transport(
    settings: RedisConnectionSettings,
    family: RedisCacheFamily,
    store: &Arc<RedisClientStore>,
    pool: Option<PoolConfig>,
) -> RedisTransport {
    RedisTransport::new(
        settings,
        family,
        Arc::new(factory()),
        Arc::clone(store),
        pool,
    )
    .expect("a transport")
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

pub(crate) fn redis_slot(key: &str) -> u16 {
    crc16::State::<crc16::XMODEM>::calculate(key.as_bytes()) & 0x3fff
}

pub(crate) fn key_in_slot_range(low: u16, high: u16) -> &'static str {
    const CANDIDATES: [&str; 12] = [
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
        "lambda", "mu",
    ];
    CANDIDATES
        .into_iter()
        .find(|key| (low..=high).contains(&redis_slot(key)))
        .expect("a key hashing into the slot range")
}

pub(crate) struct TestSource {
    tokens: Vec<(Option<String>, String)>,
    next: AtomicUsize,
}

impl RedisCredentialSource for TestSource {
    fn fetch(&self) -> Result<RedisCredentials, RedisCredentialError> {
        let index = self
            .next
            .fetch_add(1, Ordering::Relaxed)
            .min(self.tokens.len().saturating_sub(1));
        let (username, token) = self.tokens.get(index).expect("a test token");
        Ok(RedisCredentials {
            username: username.clone(),
            password: SecretString::from(token.clone()),
        })
    }

    fn refresh_after(&self) -> Duration {
        Duration::from_secs(3600)
    }
}

pub(crate) fn credential(username: Option<&str>, token: &str) -> (Option<String>, String) {
    (username.map(str::to_owned), token.to_owned())
}

pub(crate) fn source(tokens: Vec<(Option<String>, String)>) -> Arc<dyn RedisCredentialSource> {
    Arc::new(TestSource {
        tokens,
        next: AtomicUsize::new(0),
    })
}
