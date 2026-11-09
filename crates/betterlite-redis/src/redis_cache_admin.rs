pub mod connection_test;
pub mod health;
pub mod info_parse;

mod tests;

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use redis::cmd;
use tokio::runtime::Handle;

use self::connection_test::{ConnectionTestResult, connection_test_result, fresh_connection_ping};
use self::health::{ASYNC_PING, ping_commands};
use self::info_parse::{
    ClientListError, InfoValue, first_node_text, parse_client_list, parse_info,
};
use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::redis_client_reuse::breaker::BreakerOpen;
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use crate::redis_connection_settings::model::settings::RedisConnectionSettings;
use crate::redis_live_client::{LiveAsyncClient, LiveClientError, LiveClientFactory};

const SYNC_PING: &str = "sync_ping";
const SYNC_STARTUP_PING: &str = "redis_sync_ping";

pub struct RedisCacheAdmin {
    transport: Arc<RedisTransport>,
    store: Arc<RedisClientStore>,
    factory: Arc<LiveClientFactory>,
    settings: RedisConnectionSettings,
    namespace: Option<String>,
    hooks: Arc<dyn ServiceHooks>,
    gate: Arc<dyn BreakerGate>,
}

impl RedisCacheAdmin {
    // the transport already carries the store, the factory and the settings it was built with
    #[must_use]
    pub fn new(
        transport: Arc<RedisTransport>,
        namespace: Option<String>,
        hooks: Arc<dyn ServiceHooks>,
        gate: Arc<dyn BreakerGate>,
    ) -> Self {
        let store = Arc::clone(&transport.store);
        let factory = Arc::clone(&transport.factory);
        let settings = transport.settings.clone();
        Self {
            transport,
            store,
            factory,
            settings,
            namespace,
            hooks,
            gate,
        }
    }

    async fn guarded<T, F>(&self, name: &'static str, call: F) -> Result<T, RedisAdminError>
    where
        F: Future<Output = Result<T, RedisAdminError>> + Send,
    {
        let admission = self.gate.enter(name)?;
        match call.await {
            Ok(value) => {
                self.gate.exit(admission);
                Ok(value)
            }
            Err(error) => {
                self.gate.record_failure(&error);
                Err(error)
            }
        }
    }

    pub async fn ping(&self, caller: Option<&str>) -> Result<bool, RedisAdminError> {
        let started = Instant::now();
        match self.transport.async_client().await {
            Ok(client) => ping_commands(&client, self.hooks.as_ref(), caller).await,
            Err(source) => {
                let error = RedisAdminError::from(source);
                self.record_ping_failure(ASYNC_PING, started, caller, &error);
                Err(error)
            }
        }
    }

    pub fn sync_ping(&self, caller: Option<&str>) -> Result<bool, RedisAdminError> {
        let started = Instant::now();
        let reply = self.transport.sync_client.query::<String>(&cmd("PING"));
        let answer = match reply {
            Ok(answer) => answer,
            Err(source) => {
                let error = RedisAdminError::from(source);
                self.record_ping_failure(SYNC_PING, started, caller, &error);
                return Err(error);
            }
        };
        let event = ServiceEvent {
            call_type: SYNC_PING,
            duration: started.elapsed(),
            key: caller.map(str::to_owned),
        };
        self.hooks.success(&event);
        Ok(answer == "PONG")
    }

    fn record_ping_failure(
        &self,
        call_type: &'static str,
        started: Instant,
        caller: Option<&str>,
        error: &RedisAdminError,
    ) {
        let event = ServiceEvent {
            call_type,
            duration: started.elapsed(),
            key: caller.map(str::to_owned),
        };
        self.hooks.failure(&event, error);
        tracing::error!("LiteLLM Redis Cache PING: - Got exception from REDIS : {error}");
    }

    pub async fn setup_health_pings(self: &Arc<Self>) {
        // litellm creates a task for the async ping and moves on, so startup never waits on it
        let pinging = Arc::clone(self);
        drop(Handle::current().spawn(async move {
            drop(pinging.ping(None).await);
        }));
        let probing = Arc::clone(self);
        drop(
            Handle::current()
                .spawn_blocking(move || probing.sync_health_probe())
                .await,
        );
    }

    // litellm's _setup_health_pings calls redis_client.ping() directly: no success event, no caller
    fn sync_health_probe(&self) {
        let reply = self.transport.sync_client.query::<String>(&cmd("PING"));
        if let Err(source) = reply {
            let error = RedisAdminError::from(source);
            tracing::error!(error = %error, "Error connecting to Sync Redis client");
            self.hooks.failure(
                &ServiceEvent {
                    call_type: SYNC_STARTUP_PING,
                    duration: Duration::ZERO,
                    key: None,
                },
                &error,
            );
        }
    }

    pub async fn delete_cache_keys(&self, keys: &[String]) -> Result<(), RedisAdminError> {
        let namespaced: Vec<String> = keys.iter().map(|key| self.namespace_key(key)).collect();
        self.guarded("delete_cache_keys", async {
            let names: Vec<&str> = namespaced.iter().map(String::as_str).collect();
            self.delete_through(&names).await?;
            Ok(())
        })
        .await
    }

    pub async fn async_delete_cache(&self, key: &str) -> Result<i64, RedisAdminError> {
        let namespaced = self.namespace_key(key);
        self.guarded("async_delete_cache", async {
            self.delete_through(&[namespaced.as_str()]).await
        })
        .await
    }

    // redis-py's cluster delete sends nothing for an empty list, where a plain client sends a bare DEL
    async fn delete_through(&self, names: &[&str]) -> Result<i64, RedisAdminError> {
        let client = self.transport.async_client().await?;
        if names.is_empty() && matches!(&client, LiveAsyncClient::Cluster(_)) {
            return Ok(0);
        }
        Ok(client.delete(names).await?)
    }

    pub fn delete_cache(&self, key: &str) -> Result<(), RedisAdminError> {
        let namespaced = self.namespace_key(key);
        let mut command = cmd("DEL");
        command.arg(namespaced);
        self.transport.sync_client.query::<i64>(&command)?;
        Ok(())
    }

    fn namespace_key(&self, key: &str) -> String {
        check_and_fix_namespace(self.namespace.as_deref(), Some(key))
            .unwrap_or_else(|| key.to_owned())
    }

    pub fn client_list(&self) -> Result<Vec<IndexMap<String, String>>, RedisAdminError> {
        let mut command = cmd("CLIENT");
        command.arg("LIST");
        let text = self.transport.sync_client.query::<String>(&command)?;
        Ok(parse_client_list(&text)?)
    }

    pub fn info(&self) -> Result<IndexMap<String, InfoValue>, RedisAdminError> {
        let text = match self.transport.family {
            // redis-rs sends INFO to every primary and merges the replies into a map of node to text
            RedisCacheFamily::Cluster => first_node_text(
                self.transport
                    .sync_client
                    .query::<redis::Value>(&cmd("INFO"))?,
            )?,
            RedisCacheFamily::Standalone => {
                self.transport.sync_client.query::<String>(&cmd("INFO"))?
            }
        };
        Ok(parse_info(&text))
    }

    pub fn flush_cache(&self) -> Result<(), RedisAdminError> {
        // litellm keeps two names for one FLUSHALL; the one-line rule forbids a forwarding method
        self.transport
            .sync_client
            .query::<String>(&cmd("FLUSHALL"))?;
        Ok(())
    }

    pub fn flushall(&self) -> Result<(), RedisAdminError> {
        self.transport
            .sync_client
            .query::<String>(&cmd("FLUSHALL"))?;
        Ok(())
    }

    pub async fn test_connection(&self) -> ConnectionTestResult {
        let ping = match fresh_connection_ping(self.factory.as_ref(), &self.settings).await {
            Ok(answer) => Ok(answer),
            Err(error) => {
                match self.transport.family {
                    RedisCacheFamily::Standalone => {
                        tracing::error!("Redis connection test failed: {error}");
                    }
                    RedisCacheFamily::Cluster => {
                        tracing::error!("Redis Cluster connection test failed: {error}");
                    }
                }
                Err(error)
            }
        };
        connection_test_result(self.transport.family, ping)
    }

    pub async fn disconnect(&self) {
        if self.transport.family == RedisCacheFamily::Standalone {
            self.store.remove(&self.transport.cache_key);
        }
        self.transport.close_pubsub_client().await;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RedisAdminError {
    #[error(transparent)]
    ClientReuse(#[from] ClientReuseError),
    #[error(transparent)]
    LiveClient(#[from] LiveClientError),
    #[error(transparent)]
    ClientList(#[from] ClientListError),
    #[error(transparent)]
    Breaker(#[from] BreakerOpen),
}
