use std::collections::BTreeMap;
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::SecretString;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use super::fake_server::FakeServer;
use crate::redis_client_factories::RedisClient;
use crate::redis_client_factories::cluster::ClusterClient as ClusterRecord;
use crate::redis_client_factories::fields::ConnectionFields;
use crate::redis_client_factories::sentinel::SentinelClient;
use crate::redis_client_factories::url::UrlClient;
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_live_client::credentials::{
    RedisCredentialError, RedisCredentialSource, RedisCredentials,
};
use crate::redis_live_client::{LiveAsyncClient, LiveClientError, LiveClientFactory};

pub(crate) fn empty_fields() -> ConnectionFields {
    ConnectionFields {
        host: None,
        port: None,
        db: None,
        username: None,
        password: None,
        ssl: None,
        socket_timeout: None,
        socket_connect_timeout: None,
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        credential_provider_settings: None,
        ssl_password: None,
        passthrough: BTreeMap::new(),
    }
}

pub(crate) fn direct_fields(host: &str, port: u16) -> ConnectionFields {
    let mut fields = empty_fields();
    fields.host = Some(host.to_owned());
    fields.port = Some(i64::from(port));
    fields
}

pub(crate) fn direct_client(server: &FakeServer) -> RedisClient {
    RedisClient::Direct(direct_fields(&server.host(), server.port()))
}

pub(crate) fn url_client(url: String) -> UrlClient {
    UrlClient {
        url: SecretString::from(url),
        username: None,
        socket_timeout: None,
        socket_connect_timeout: None,
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        credential_provider_settings: None,
        passthrough: BTreeMap::new(),
    }
}

pub(crate) fn cluster_record(servers: &[&FakeServer]) -> ClusterRecord {
    ClusterRecord {
        startup_nodes: servers
            .iter()
            .map(|server| ClusterNodeConfig {
                host: server.host(),
                port: i64::from(server.port()),
            })
            .collect(),
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

pub(crate) fn sentinel_record(monitors: &[SocketAddr], service: &str) -> SentinelClient {
    SentinelClient {
        sentinel_nodes: monitors
            .iter()
            .map(|monitor| (monitor.ip().to_string(), i64::from(monitor.port())))
            .collect(),
        service_name: service.to_owned(),
        connection_kwargs: empty_fields(),
        monitor_password: None,
    }
}

pub(crate) fn closed_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    drop(listener);
    address
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
    refresh: Duration,
}

impl TestSource {
    pub(crate) fn new(tokens: Vec<(Option<String>, String)>) -> Self {
        Self {
            tokens,
            next: AtomicUsize::new(0),
            refresh: Duration::from_secs(3600),
        }
    }

    pub(crate) fn refreshing(mut self, refresh: Duration) -> Self {
        self.refresh = refresh;
        self
    }
}

impl RedisCredentialSource for TestSource {
    fn fetch(&self) -> Result<RedisCredentials, RedisCredentialError> {
        let index = self.next.fetch_add(1, Ordering::Relaxed);
        let capped = index.min(self.tokens.len().saturating_sub(1));
        let (username, token) = &self.tokens[capped];
        Ok(RedisCredentials {
            username: username.clone(),
            password: SecretString::from(token.clone()),
        })
    }

    fn refresh_after(&self) -> Duration {
        self.refresh
    }
}

pub(crate) struct FailingSource;

impl RedisCredentialSource for FailingSource {
    fn fetch(&self) -> Result<RedisCredentials, RedisCredentialError> {
        Err(RedisCredentialError::Fetch(io::Error::other(
            "no credential is available",
        )))
    }

    fn refresh_after(&self) -> Duration {
        Duration::from_secs(60)
    }
}

pub(crate) fn credential(username: Option<&str>, token: &str) -> (Option<String>, String) {
    (username.map(str::to_owned), token.to_owned())
}

pub(crate) fn source(tokens: Vec<(Option<String>, String)>) -> Arc<dyn RedisCredentialSource> {
    Arc::new(TestSource::new(tokens))
}

pub(crate) fn factory(source: Option<Arc<dyn RedisCredentialSource>>) -> LiveClientFactory {
    LiveClientFactory::new(source)
}

pub(crate) async fn connected(server: &FakeServer) -> LiveAsyncClient {
    factory(None)
        .open_async(&direct_client(server), None)
        .await
        .expect("a direct connection")
}

pub(crate) async fn open_error(
    factory: &LiveClientFactory,
    client: &RedisClient,
) -> LiveClientError {
    match factory.open_async(client, None).await {
        Ok(_connected) => panic!("the connection should fail"),
        Err(error) => error,
    }
}

pub(crate) fn endpoint_error(factory: &LiveClientFactory, client: &RedisClient) -> LiveClientError {
    match factory.blocking_endpoint(client) {
        Ok(_endpoint) => panic!("the endpoint should fail"),
        Err(error) => error,
    }
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(crate) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let guard = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .set_default();
        (logs, guard)
    }

    pub(crate) fn lines(&self) -> Vec<String> {
        let bytes = self.0.lock().expect("the log buffer").clone();
        String::from_utf8(bytes)
            .expect("UTF-8 log output")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    pub(crate) fn warnings(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|line| line.contains(" WARN "))
            .collect()
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut logged = self
            .0
            .lock()
            .map_err(|_poisoned| io::Error::other("log buffer poisoned"))?;
        logged.extend_from_slice(buf);
        drop(logged);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn assert_no_secret(error: &LiveClientError, secret: &str) {
    let text = format!("{error} {error:?}");
    assert!(!text.contains(secret), "an error carries {secret}: {text}");
}
