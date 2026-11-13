use std::collections::BTreeMap;
use std::fmt::Display;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::batch::MgetValues;
use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::request_batch::batches::RequestRedisBatches;
use betterlite_redis::request_batch::scope::{ACTIVE_REQUEST_BATCHES, request_redis_batch_scope};
use serde_json::{Value as Json, json};
use thiserror::Error;

use super::input::{CacheSpec, Call};
use super::server::{FakeServer, connect_cache};
use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Error)]
pub(crate) enum Failure {
    #[error("cannot start the helper runtime: {0}")]
    Runtime(#[source] io::Error),
    #[error("cannot start the fake Redis server: {0}")]
    Server(#[from] io::Error),
    #[error("cannot connect the fake Redis cache: {0}")]
    Cache(#[source] RedisCacheError),
}

pub(crate) fn invalid<M>(message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

pub(crate) enum Handle {
    Mget(BatchResult<MgetValues>),
    Increment(BatchResult<f64>),
}

#[derive(Clone, Copy)]
pub(crate) enum Target {
    Active,
    Stored(usize),
}

pub(crate) enum Prefetch {
    Arm { slot: String, value: String },
    Take { slot: String },
}

pub(crate) struct BatchEntry {
    pub(crate) batch: Arc<RedisBatch>,
    pub(crate) kind: &'static str,
    pub(crate) caches: Vec<usize>,
}

pub(crate) struct Scenario<'call> {
    pub(crate) call: &'call Call,
    pub(crate) deadline: Duration,
    pub(crate) servers: BTreeMap<String, FakeServer>,
    pub(crate) caches: Vec<Arc<RedisCache>>,
    pub(crate) requests: Vec<Arc<RequestRedisBatches>>,
    pub(crate) batches: Vec<BatchEntry>,
    pub(crate) handles: Vec<Option<Handle>>,
    pub(crate) snapshots: Vec<Json>,
    pub(crate) awaited: Vec<Json>,
    pub(crate) nested: Vec<Json>,
    pub(crate) prefetch: Vec<Json>,
}

pub(crate) async fn run(call: &Call) -> HelperResult {
    let deadline = Duration::from_millis(call.deadline_millis);
    let mut scenario = Scenario::start(call, deadline).await?;
    let walked = if call.in_scope {
        let (request, walked) = request_redis_batch_scope(deadline, async {
            if let Ok(active) = ACTIVE_REQUEST_BATCHES.try_with(Arc::clone) {
                scenario.record_request(&active);
            }
            scenario.walk().await
        })
        .await;
        scenario.record_request(&request);
        walked
    } else {
        scenario.walk().await
    };
    walked?;
    Ok(scenario.report())
}

impl<'call> Scenario<'call> {
    async fn start(call: &'call Call, deadline: Duration) -> Result<Self, Box<HelperError>> {
        let mut scenario = Self {
            call,
            deadline,
            servers: BTreeMap::new(),
            caches: Vec::new(),
            requests: Vec::new(),
            batches: Vec::new(),
            handles: Vec::new(),
            snapshots: Vec::new(),
            awaited: Vec::new(),
            nested: Vec::new(),
            prefetch: Vec::new(),
        };
        for spec in &call.caches {
            scenario.connect(spec).await?;
        }
        Ok(scenario)
    }

    async fn connect(&mut self, spec: &CacheSpec) -> Result<(), Box<HelperError>> {
        if !self.servers.contains_key(&spec.setting) {
            let server = FakeServer::start().await.map_err(invalid)?;
            if let Some(scripts) = self.call.replies.get(&spec.setting) {
                for (command, replies) in scripts {
                    for reply in replies {
                        server.script(command, reply.clone());
                    }
                }
            }
            self.servers.insert(spec.setting.clone(), server);
        }
        let cache = {
            let server = self
                .servers
                .get(&spec.setting)
                .ok_or_else(|| invalid(format!("unknown setting {}", spec.setting)))?;
            connect_cache(server, spec.namespace.as_deref())
                .await
                .map_err(invalid)?
        };
        self.caches.push(cache);
        Ok(())
    }

    async fn walk(&mut self) -> Result<(), Box<HelperError>> {
        let call = self.call;
        for step in &call.steps {
            self.step(step).await?;
        }
        Ok(())
    }

    pub(crate) fn record_request(&mut self, request: &Arc<RequestRedisBatches>) -> usize {
        if let Some(index) = self
            .requests
            .iter()
            .position(|held| Arc::ptr_eq(held, request))
        {
            return index;
        }
        self.requests.push(Arc::clone(request));
        self.requests.len().saturating_sub(1)
    }

    pub(crate) fn record_batch(
        &mut self,
        batch: &Arc<RedisBatch>,
        kind: &'static str,
        cache: usize,
    ) -> usize {
        if let Some(index) = self
            .batches
            .iter()
            .position(|entry| Arc::ptr_eq(&entry.batch, batch))
        {
            if let Some(entry) = self.batches.get_mut(index)
                && !entry.caches.contains(&cache)
            {
                entry.caches.push(cache);
            }
            return index;
        }
        self.batches.push(BatchEntry {
            batch: Arc::clone(batch),
            kind,
            caches: vec![cache],
        });
        self.batches.len().saturating_sub(1)
    }

    pub(crate) fn cache_arc(&self, index: usize) -> Result<Arc<RedisCache>, Box<HelperError>> {
        self.caches
            .get(index)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown cache {index}")))
    }

    pub(crate) fn request_arc(
        &self,
        index: usize,
    ) -> Result<Arc<RequestRedisBatches>, Box<HelperError>> {
        self.requests
            .get(index)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown request {index}")))
    }

    pub(crate) fn active_request() -> Result<Arc<RequestRedisBatches>, Box<HelperError>> {
        ACTIVE_REQUEST_BATCHES
            .try_with(Arc::clone)
            .map_err(|_outside_scope| invalid("the step needs an active request_redis_batch_scope"))
    }

    pub(crate) fn touch_active_request(&mut self) -> Result<(), Box<HelperError>> {
        let request = Self::active_request()?;
        self.record_request(&request);
        Ok(())
    }

    pub(crate) fn state(&self) -> Json {
        let batches: Vec<Json> = self
            .batches
            .iter()
            .map(|entry| {
                json!({
                    "kind": entry.kind,
                    "caches": entry.caches,
                    "pending": entry.batch.pending(),
                    "flushes": entry.batch.flushes(),
                })
            })
            .collect();
        json!({
            "batches": batches,
            "requests": self.requests.len(),
        })
    }

    fn report(self) -> Json {
        let state = self.state();
        json!({
            "state": state,
            "snapshots": self.snapshots,
            "awaited": self.awaited,
            "nested": self.nested,
            "prefetch": self.prefetch,
        })
    }
}
