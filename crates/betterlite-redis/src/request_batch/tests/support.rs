use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

use crate::redis_cache::RedisCache;
use crate::redis_cache::connect::{RedisCacheOptions, connect};
use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_connection_settings::model::scalars::{FloatOrText, IntOrText};
use crate::redis_connection_settings::model::settings::RedisOverrides;
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::tests::fake_server::FakeServer;

const BATCH_COMMANDS: [&str; 7] = [
    "DEL",
    "EVALSHA",
    "EXPIRE",
    "INCRBYFLOAT",
    "MGET",
    "SCRIPT",
    "SET",
];
const DEFAULT_TTL: i64 = 60;
const SOCKET_TIMEOUT_SECONDS: f64 = 1.0;

struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct AllowGate;

impl BreakerGate for AllowGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

fn blank_overrides() -> RedisOverrides {
    RedisOverrides {
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
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        gcp_service_account: None,
        gcp_ssl_ca_certs: None,
        azure_redis_ad_token: None,
        azure_client_id: None,
        azure_tenant_id: None,
        azure_client_secret: None,
        aws_iam_auth: None,
        aws_iam_user_name: None,
        aws_iam_cache_name: None,
        aws_iam_region: None,
        aws_iam_serverless: None,
        passthrough: BTreeMap::new(),
    }
}

pub(crate) async fn cache(server: &FakeServer, namespace: Option<&str>) -> Arc<RedisCache> {
    let mut overrides = blank_overrides();
    overrides.host = Some(server.host());
    overrides.port = Some(IntOrText::Value(i64::from(server.port())));
    overrides.socket_timeout = Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS));
    overrides.socket_connect_timeout = Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS));
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: namespace.map(str::to_owned),
        default_ttl: DEFAULT_TTL,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate: Arc::new(AllowGate),
    };
    let env = BTreeMap::<String, String>::new();
    Arc::new(connect(options, &env).await.expect("a connected cache"))
}

pub(crate) fn command_names(server: &FakeServer) -> Vec<String> {
    server
        .commands()
        .into_iter()
        .map(|command| command.name)
        .filter(|name| BATCH_COMMANDS.contains(&name.as_str()))
        .collect()
}
