pub mod error;
mod lpop;
mod lpop_grouped;
pub mod results;
mod rpush;
mod rpush_pipeline;
mod tests;
mod trim;

use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::protocol::RedisScalar;
use crate::cache_primitives::version::RedisVersionValue;
use crate::redis_client_reuse::breaker::BreakerAdmission;
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::transport::RedisTransport;

use self::error::RedisListOperationsError;
use self::results::{LpopPipelineEntry, LpopResult};

pub(crate) fn namespaced_key(lists: &RedisListOperations, key: &str) -> String {
    check_and_fix_namespace(lists.settings.namespace.as_deref(), Some(key))
        .unwrap_or_else(|| key.to_owned())
}

pub(crate) struct Call {
    pub(crate) name: &'static str,
    pub(crate) log: &'static str,
}

pub type ListResult<T> = Result<T, RedisListOperationsError>;
pub type RpushList = [RedisPipelineRpushOperation];
pub type LpopList = [RedisPipelineLpopOperation];
pub type LpopEntries = Vec<LpopPipelineEntry>;

pub struct RedisListOperationsSettings {
    pub namespace: Option<String>,
    pub redis_version: RedisVersionValue,
    pub default_major_version: u32,
}

pub struct RedisListOperations {
    settings: RedisListOperationsSettings,
    transport: Arc<RedisTransport>,
    hooks: Arc<dyn ServiceHooks>,
    gate: Arc<dyn BreakerGate>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RedisPipelineRpushOperation {
    pub key: String,
    pub values: Vec<RedisScalar>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisPipelineLpopOperation {
    pub key: String,
    pub count: Option<i64>,
}

impl RedisListOperations {
    #[must_use]
    pub fn new(
        settings: RedisListOperationsSettings,
        transport: Arc<RedisTransport>,
        hooks: Arc<dyn ServiceHooks>,
        gate: Arc<dyn BreakerGate>,
    ) -> Self {
        Self {
            settings,
            transport,
            hooks,
            gate,
        }
    }

    pub async fn async_rpush(&self, key: &str, values: &[RedisScalar]) -> ListResult<i64> {
        let call = Call {
            name: "async_rpush",
            log: "LiteLLM Redis Cache RPUSH: - Got exception from REDIS",
        };
        rpush::async_rpush(self, call, key, values).await
    }

    pub async fn async_rpush_and_trim(
        &self,
        key: &str,
        values: &[RedisScalar],
        max_len: i64,
    ) -> ListResult<i64> {
        let call = Call {
            name: "async_rpush_and_trim",
            log: "LiteLLM Redis Cache RPUSH+LTRIM: - Got exception from REDIS",
        };
        trim::async_rpush_and_trim(self, call, key, values, max_len).await
    }

    pub async fn async_rpush_pipeline(&self, rpush_list: &RpushList) -> ListResult<Vec<i64>> {
        let call = Call {
            name: "async_rpush_pipeline",
            log: "LiteLLM Redis Caching: async_rpush_pipeline() - Got exception from REDIS",
        };
        rpush_pipeline::async_rpush_pipeline(self, call, rpush_list).await
    }

    pub async fn async_lpop(&self, key: &str, count: Option<i64>) -> ListResult<LpopResult> {
        let call = Call {
            name: "async_lpop",
            log: "LiteLLM Redis Cache LPOP: - Got exception from REDIS",
        };
        lpop::async_lpop(self, call, key, count).await
    }

    pub async fn async_lpop_pipeline(&self, lpop_list: &LpopList) -> ListResult<LpopEntries> {
        let call = Call {
            name: "async_lpop_pipeline",
            log: "LiteLLM Redis Caching: async_lpop_pipeline() - Got exception from REDIS",
        };
        lpop::async_lpop_pipeline(self, call, lpop_list).await
    }

    // litellm raises from the guarded method, so a failed call records the failure and never reaches exit
    async fn perform<T, F>(
        &self,
        admission: BreakerAdmission,
        call: Call,
        key: Option<&str>,
        body: F,
    ) -> ListResult<T>
    where
        F: Future<Output = ListResult<T>>,
    {
        let started = Instant::now();
        let outcome = body.await;
        let event = ServiceEvent {
            call_type: call.name,
            duration: started.elapsed(),
            key: key.map(|raw| namespaced_key(self, raw)),
        };
        match outcome {
            Ok(value) => {
                self.hooks.success(&event);
                self.gate.exit(admission);
                Ok(value)
            }
            Err(error) => {
                self.hooks.failure(&event, &error);
                tracing::error!(error = %error, "{}", call.log);
                self.gate.record_failure(&error);
                Err(error)
            }
        }
    }

    fn admit(
        &self,
        name: &'static str,
        empty: bool,
    ) -> Result<Option<BreakerAdmission>, RedisListOperationsError> {
        let admission = self.gate.enter(name)?;
        if empty {
            self.gate.exit(admission);
            return Ok(None);
        }
        Ok(Some(admission))
    }
}
