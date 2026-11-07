use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

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
    let mut crc: u16 = 0;
    for byte in key.as_bytes() {
        crc ^= u16::from(*byte).wrapping_shl(8);
        for _bit in 0_u32..8_u32 {
            crc = if crc & 0x8000 != 0 {
                crc.wrapping_shl(1) ^ 0x1021
            } else {
                crc.wrapping_shl(1)
            };
        }
    }
    crc & 0x3fff
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

#[derive(Clone, Debug)]
pub(crate) struct CapturedEvent {
    pub(crate) level: tracing::Level,
    pub(crate) fields: Vec<(String, String)>,
}

impl CapturedEvent {
    pub(crate) fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _value)| field == name)
            .map(|(_field, value)| value.as_str())
    }

    pub(crate) fn text(&self) -> String {
        let fields: Vec<String> = self
            .fields
            .iter()
            .map(|(field, value)| format!("{field}={value}"))
            .collect();
        fields.join(" ")
    }
}

#[derive(Clone, Default)]
pub(crate) struct Capture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl Capture {
    pub(crate) fn warnings(&self) -> Vec<CapturedEvent> {
        let events = self.events.lock().expect("the captured events");
        events
            .iter()
            .filter(|event| event.level == tracing::Level::WARN)
            .cloned()
            .collect()
    }
}

struct FieldCollector {
    fields: Vec<(String, String)>,
}

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .push((field.name().to_owned(), value.to_owned()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields
            .push((field.name().to_owned(), format!("{value:?}")));
    }
}

impl Subscriber for Capture {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut collector = FieldCollector { fields: Vec::new() };
        event.record(&mut collector);
        let mut events = self.events.lock().expect("the captured events");
        events.push(CapturedEvent {
            level: *event.metadata().level(),
            fields: collector.fields,
        });
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

pub(crate) fn assert_no_secret(error: &LiveClientError, secret: &str) {
    let text = format!("{error} {error:?}");
    assert!(!text.contains(secret), "an error carries {secret}: {text}");
}
