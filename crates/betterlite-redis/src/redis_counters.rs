pub mod counts;
pub mod float_increment;
pub mod floor;
pub mod guard;
pub mod increment;
pub mod reply;
pub mod scan;
pub mod scripts;
pub mod set_max;

mod tests;

use std::result::Result as StdResult;
use std::sync::Arc;
use std::time::Instant;

use redis::{Value, cmd};

use crate::cache_primitives::decode::DecodeError;
use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::pipeline_increment::{
    IncrementOperation, PipelineStep, float_replies, plan_pipeline_steps,
};
use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::redis_client_reuse::breaker::BreakerOpen;
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::transport::RedisTransport;
use crate::redis_live_client::LiveClientError;

use self::guard::finish_guard;

pub type Counts = Vec<Option<i64>>;

pub type Result<T> = StdResult<T, RedisCountersError>;

pub struct RedisCounters {
    pub transport: Arc<RedisTransport>,
    pub namespace: Option<String>,
    pub default_ttl: i64,
    pub hooks: Arc<dyn ServiceHooks>,
    pub gate: Arc<dyn BreakerGate>,
}

impl RedisCounters {
    #[must_use]
    pub fn new(
        transport: Arc<RedisTransport>,
        namespace: Option<String>,
        default_ttl: i64,
        hooks: Arc<dyn ServiceHooks>,
        gate: Arc<dyn BreakerGate>,
    ) -> Self {
        Self {
            transport,
            namespace,
            default_ttl,
            hooks,
            gate,
        }
    }

    pub fn increment_cache(&self, key: &str, value: i64, ttl: Option<f64>) -> Result<i64> {
        let set_ttl = increment::resolve_ttl(ttl, self.default_ttl)?;
        let namespaced = check_and_fix_namespace(self.namespace.as_deref(), Some(key))
            .unwrap_or_else(|| key.to_owned());
        let started = Instant::now();
        let outcome = increment::increment_steps(self, &namespaced, value, set_ttl);
        if let Err(error) = &outcome {
            self.hooks.failure(
                &ServiceEvent {
                    call_type: "increment_cache",
                    duration: started.elapsed(),
                    key: Some(namespaced),
                },
                error,
            );
            tracing::error!(
                error = %error,
                "LiteLLM Redis Caching: increment_cache() - Got exception from REDIS"
            );
        }
        outcome
    }

    pub fn increment_with_floor(&self, key: &str, value: i64, ttl: i64) -> Result<i64> {
        let admission = self.gate.enter("increment_with_floor")?;
        let outcome = floor::sync_floor(self, key, value, ttl);
        finish_guard(self.gate.as_ref(), admission, outcome)
    }

    pub async fn async_increment_with_floor(&self, key: &str, value: i64, ttl: i64) -> Result<i64> {
        let admission = self.gate.enter("async_increment_with_floor")?;
        let outcome = floor::eval_floor(self, key, value, ttl).await;
        finish_guard(self.gate.as_ref(), admission, outcome)
    }

    pub fn batch_get_counts(&self, key_list: &[String]) -> Result<Counts> {
        let admission = self.gate.enter("batch_get_counts")?;
        let outcome = counts::batch_get_counts(self, key_list);
        finish_guard(self.gate.as_ref(), admission, outcome)
    }

    pub async fn async_batch_get_counts(&self, key_list: &[String]) -> Result<Counts> {
        let admission = self.gate.enter("async_batch_get_counts")?;
        let outcome = counts::read_counts(self, key_list).await;
        finish_guard(self.gate.as_ref(), admission, outcome)
    }

    pub async fn async_scan_iter(&self, pattern: &str, count: usize) -> Result<Vec<Vec<u8>>> {
        let started = Instant::now();
        let admission = self.gate.enter(scan::SCAN_OPERATION)?;
        let outcome = finish_guard(
            self.gate.as_ref(),
            admission,
            scan::scan(self, pattern, count).await,
        );
        let event = ServiceEvent {
            call_type: scan::SCAN_OPERATION,
            duration: started.elapsed(),
            key: None,
        };
        match &outcome {
            Ok(_keys) => self.hooks.success(&event),
            Err(error) => self.hooks.failure(&event, error),
        }
        outcome
    }

    #[must_use]
    pub fn async_register_script(&self, script: &str) -> scripts::RegisteredScript {
        scripts::RegisteredScript {
            source: script.to_owned(),
            sha1: redis::Script::new(script).get_hash().to_owned(),
            transport: Arc::clone(&self.transport),
            namespace: self.namespace.clone(),
        }
    }

    pub async fn incr_by_float_with_ttl(
        &self,
        key: &str,
        value: f64,
        ttl: Option<i64>,
        refresh_ttl: bool,
    ) -> Result<f64> {
        let client = self.transport.async_client().await?;
        match ttl {
            None => {
                let mut command = cmd("INCRBYFLOAT");
                command.arg(key).arg(value);
                let reply: Value = client.query(&command).await?;
                float_increment::float_reply(reply, "incr_by_float_with_ttl")
            }
            Some(ttl_seconds) => {
                float_increment::increment_with_followup(
                    &client,
                    key,
                    value,
                    ttl_seconds,
                    refresh_ttl,
                )
                .await
            }
        }
    }

    pub async fn pipeline_increment(
        &self,
        increment_list: &[IncrementOperation],
    ) -> Result<Vec<f64>> {
        if increment_list.is_empty() {
            return Ok(Vec::new());
        }
        let steps = plan_pipeline_steps(increment_list, self.namespace.as_deref());
        let client = self.transport.async_client().await?;
        let mut pipeline = client.pipeline(false);
        for step in &steps {
            float_increment::add_pipeline_step(&mut pipeline, step);
        }
        let replies = pipeline.execute().await?;
        let mut floats: Vec<Option<f64>> = Vec::with_capacity(replies.len());
        for (step, reply) in steps.iter().zip(replies.iter()) {
            match step {
                PipelineStep::IncrByFloat { .. } => floats.push(Some(
                    float_increment::float_reply(reply.clone(), "pipeline_increment")?,
                )),
                PipelineStep::Expire { .. } => floats.push(None),
            }
        }
        Ok(float_replies(&floats))
    }

    pub async fn async_set_max(
        &self,
        key: &str,
        value: f64,
        ttl: Option<i64>,
    ) -> Result<Option<f64>> {
        let admission = self.gate.enter("async_set_max")?;
        let outcome = set_max::set_max_body(self, key, value, ttl).await;
        finish_guard(self.gate.as_ref(), admission, outcome)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RedisCountersError {
    #[error(transparent)]
    Client(#[from] LiveClientError),
    #[error(transparent)]
    Reuse(#[from] ClientReuseError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Breaker(#[from] BreakerOpen),
    #[error("redis returned an unexpected reply for {operation}")]
    UnexpectedReply { operation: &'static str },
    #[error("the ttl is not a finite number of seconds")]
    InvalidTtl,
    #[error("the value is not a finite number")]
    NonFiniteValue,
}
