use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{Arc, Mutex};

use crate::cache_primitives::version::RedisVersionValue;
use crate::list_operations::{RedisListOperations, RedisListOperationsSettings};
use crate::redis_cache::RedisCache;
use crate::redis_cache_admin::RedisCacheAdmin;
use crate::redis_cache_values::{RedisCacheValues, RedisCacheValuesSettings};
use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::settings::RedisConnectionSettings;
use crate::redis_counters::RedisCounters;
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::tests::fake_server::{ClusterSlots, FakeServer};

pub(crate) const DEFAULT_TTL: i64 = 60;

#[derive(Clone, Default)]
pub(crate) struct Log {
    entries: Arc<Mutex<Vec<String>>>,
}

impl Log {
    fn push(&self, entry: String) {
        match self.entries.lock() {
            Ok(mut entries) => entries.push(entry),
            Err(poisoned) => poisoned.into_inner().push(entry),
        }
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        match self.entries.lock() {
            Ok(entries) => entries.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub(crate) fn for_call(&self, call_type: &str) -> Vec<String> {
        let success = format!("success:{call_type}");
        let failure = format!("failure:{call_type}");
        self.entries()
            .into_iter()
            .filter(|entry| entry.starts_with(&success) || entry.starts_with(&failure))
            .collect()
    }
}

struct TestGate {
    log: Log,
}

impl BreakerGate for TestGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        self.log.push(format!("enter:{name}"));
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, admission: BreakerAdmission) {
        self.log.push(format!("exit:{}", *admission.as_ref()));
    }

    fn record_failure(&self, error: &(dyn Error + Send + Sync)) {
        self.log.push(format!("failure:{error}"));
    }

    fn swallowed(&self, _admission: &BreakerAdmission, error: &(dyn Error + Send + Sync)) {
        self.log.push(format!("swallowed:{error}"));
    }
}

struct TestHooks {
    log: Log,
}

impl ServiceHooks for TestHooks {
    fn success(&self, event: &ServiceEvent) {
        self.log.push(format!("success:{}", event.call_type));
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn Error + Send + Sync)) {
        self.log
            .push(format!("failure:{}:{error}", event.call_type));
    }
}

pub(crate) struct Harness {
    pub(crate) cache: Arc<RedisCache>,
    pub(crate) hooks: Log,
    pub(crate) gate: Log,
}

fn blank_settings() -> RedisConnectionSettings {
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

fn settings(server: &FakeServer, family: RedisCacheFamily) -> RedisConnectionSettings {
    let mut settings = blank_settings();
    match family {
        RedisCacheFamily::Standalone => {
            settings.host = Some(server.host());
            settings.port = Some(i64::from(server.port()));
        }
        RedisCacheFamily::Cluster => {
            settings.startup_nodes = Some(vec![ClusterNodeConfig {
                host: server.host(),
                port: i64::from(server.port()),
            }]);
        }
    }
    settings
}

fn topology(server: &FakeServer) -> ClusterSlots {
    vec![(0, 16383, vec![(server.host(), server.port())])]
}

pub(crate) fn harness(server: &FakeServer, namespace: Option<&str>) -> Harness {
    make_harness(
        settings(server, RedisCacheFamily::Standalone),
        RedisCacheFamily::Standalone,
        namespace,
    )
}

pub(crate) fn cluster_harness(server: &FakeServer, namespace: Option<&str>) -> Harness {
    server.set_cluster_slots(topology(server));
    make_harness(
        settings(server, RedisCacheFamily::Cluster),
        RedisCacheFamily::Cluster,
        namespace,
    )
}

pub(crate) fn unreachable_harness() -> Harness {
    let mut settings = blank_settings();
    settings.host = Some("127.0.0.1".to_owned());
    settings.port = Some(1);
    make_harness(settings, RedisCacheFamily::Standalone, Some("ns"))
}

fn make_harness(
    settings: RedisConnectionSettings,
    family: RedisCacheFamily,
    namespace: Option<&str>,
) -> Harness {
    let hooks = Log::default();
    let gate = Log::default();
    let service_hooks: Arc<dyn ServiceHooks> = Arc::new(TestHooks { log: hooks.clone() });
    let breaker_gate: Arc<dyn BreakerGate> = Arc::new(TestGate { log: gate.clone() });
    let transport = Arc::new(
        RedisTransport::new(
            settings,
            family,
            Arc::new(LiveClientFactory::new(None)),
            Arc::new(RedisClientStore::new()),
            None,
        )
        .expect("a redis transport"),
    );
    let cache = assemble(transport, namespace, service_hooks, breaker_gate);
    Harness {
        cache: Arc::new(cache),
        hooks,
        gate,
    }
}

fn assemble(
    transport: Arc<RedisTransport>,
    namespace: Option<&str>,
    hooks: Arc<dyn ServiceHooks>,
    gate: Arc<dyn BreakerGate>,
) -> RedisCache {
    let namespace_value = namespace.map(str::to_owned);
    let values = values_slice(
        Arc::clone(&transport),
        namespace,
        Arc::clone(&hooks),
        Arc::clone(&gate),
    );
    let counters = counters_slice(&transport, namespace, Arc::clone(&hooks), Arc::clone(&gate));
    let admin = admin_slice(
        Arc::clone(&transport),
        namespace,
        Arc::clone(&hooks),
        Arc::clone(&gate),
    );
    let lists = lists_slice(&transport, namespace, Arc::clone(&hooks), Arc::clone(&gate));
    RedisCache {
        transport,
        values,
        counters,
        admin,
        lists,
        namespace: namespace_value,
        default_ttl: DEFAULT_TTL,
        redis_major_version: 7,
        hooks,
        gate,
    }
}

fn values_slice(
    transport: Arc<RedisTransport>,
    namespace: Option<&str>,
    hooks: Arc<dyn ServiceHooks>,
    gate: Arc<dyn BreakerGate>,
) -> RedisCacheValues {
    RedisCacheValues::new(
        RedisCacheValuesSettings {
            namespace: namespace.map(str::to_owned),
            default_ttl: Some(DEFAULT_TTL),
            redis_flush_size: None,
        },
        transport,
        hooks,
        gate,
    )
}

fn counters_slice(
    transport: &Arc<RedisTransport>,
    namespace: Option<&str>,
    hooks: Arc<dyn ServiceHooks>,
    gate: Arc<dyn BreakerGate>,
) -> RedisCounters {
    RedisCounters::new(
        Arc::clone(transport),
        namespace.map(str::to_owned),
        DEFAULT_TTL,
        hooks,
        gate,
    )
}

fn admin_slice(
    transport: Arc<RedisTransport>,
    namespace: Option<&str>,
    hooks: Arc<dyn ServiceHooks>,
    gate: Arc<dyn BreakerGate>,
) -> Arc<RedisCacheAdmin> {
    Arc::new(RedisCacheAdmin::new(
        transport,
        namespace.map(str::to_owned),
        hooks,
        gate,
    ))
}

fn lists_slice(
    transport: &Arc<RedisTransport>,
    namespace: Option<&str>,
    hooks: Arc<dyn ServiceHooks>,
    gate: Arc<dyn BreakerGate>,
) -> RedisListOperations {
    RedisListOperations::new(
        RedisListOperationsSettings {
            namespace: namespace.map(str::to_owned),
            redis_version: RedisVersionValue::Text("7.0.0".to_owned()),
            default_major_version: 7,
        },
        Arc::clone(transport),
        hooks,
        gate,
    )
}
