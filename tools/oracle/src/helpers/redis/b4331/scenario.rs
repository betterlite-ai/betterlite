use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::batch::MgetValues;
use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::batches::RequestRedisBatches;
use betterlite_redis::request_batch::registry::drain_post_call_redis_batches;
use betterlite_redis::request_batch::scope::{
    active_post_call_redis_batch, active_request_redis_batch, active_request_redis_batches,
    flush_post_call_redis_batches, request_redis_batch_scope,
};
use serde_json::{Value as Json, json};
use tokio::time::sleep;

use super::input::{CacheSpec, Call, PostCall, Step};
use super::server::{FakeServer, connect_cache};
use crate::helpers::{HelperError, HelperResult};

const BATCH_COMMANDS: [&str; 7] = [
    "DEL",
    "EVALSHA",
    "EXPIRE",
    "INCRBYFLOAT",
    "MGET",
    "SCRIPT",
    "SET",
];

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

enum Handle {
    Mget(BatchResult<MgetValues>),
    Increment(BatchResult<f64>),
}

#[derive(Clone, Copy)]
enum Target {
    Active,
    Stored(usize),
}

enum Prefetch {
    Arm { slot: String, value: String },
    Take { slot: String },
    Armed { slot: String },
}

struct BatchEntry {
    batch: Arc<RedisBatch>,
    kind: &'static str,
    caches: Vec<usize>,
}

struct Scenario<'call> {
    call: &'call Call,
    deadline: Duration,
    servers: BTreeMap<String, FakeServer>,
    caches: Vec<Arc<RedisCache>>,
    requests: Vec<Arc<RequestRedisBatches>>,
    batches: Vec<BatchEntry>,
    handles: Vec<Option<Handle>>,
    steps: Vec<Json>,
}

pub(crate) async fn run(call: &Call) -> HelperResult {
    let deadline = Duration::from_millis(call.deadline_millis);
    let mut scenario = Scenario::start(call, deadline).await?;
    let walked = if call.in_scope {
        let (request, walked) = request_redis_batch_scope(deadline, async {
            if let Some(active) = active_request_redis_batches() {
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
            steps: Vec::new(),
        };
        for spec in &call.caches {
            scenario.connect(spec).await?;
        }
        Ok(scenario)
    }

    async fn connect(&mut self, spec: &CacheSpec) -> Result<(), Box<HelperError>> {
        if !self.servers.contains_key(&spec.setting) {
            let server = self.start_server(&spec.setting).await?;
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

    async fn start_server(&self, setting: &str) -> Result<FakeServer, Box<HelperError>> {
        let server = FakeServer::start()
            .await
            .map_err(|error| invalid(error.to_string()))?;
        if let Some(scripts) = self.call.replies.get(setting) {
            for (command, replies) in scripts {
                for reply in replies {
                    server.script(command, reply.clone());
                }
            }
        }
        Ok(server)
    }

    async fn walk(&mut self) -> Result<(), Box<HelperError>> {
        let call = self.call;
        for step in &call.steps {
            self.step(step).await?;
        }
        Ok(())
    }

    async fn step(&mut self, step: &Step) -> Result<(), Box<HelperError>> {
        match step {
            Step::Batch { cache, keys } => self.declare_batch(*cache, keys),
            Step::PostCall { call } => self.declare_post_call(Target::Active, call),
            Step::PostCallOn { request, call } => {
                self.declare_post_call(Target::Stored(*request), call)
            }
            Step::Await { handle } => self.settle(*handle).await,
            Step::FlushPostCall => self.flush_post_call().await,
            Step::FlushAll => self.flush_all().await,
            Step::Sleep { millis } => self.pause(*millis).await,
            Step::Nested { cache } => self.nested(*cache).await,
            Step::Drain => self.drain().await,
            Step::CreateRequest => self.create_request().await,
            Step::PrefetchArm { slot, value } => self.prefetch(Prefetch::Arm {
                slot: slot.clone(),
                value: value.clone(),
            }),
            Step::PrefetchTake { slot } => self.prefetch(Prefetch::Take { slot: slot.clone() }),
            Step::PrefetchArmed { slot } => self.prefetch(Prefetch::Armed { slot: slot.clone() }),
            Step::Snapshot => {
                let snapshot = self.snapshot();
                self.steps.push(snapshot);
                Ok(())
            }
        }
    }

    fn declare_batch(&mut self, cache: usize, keys: &[String]) -> Result<(), Box<HelperError>> {
        let cached = self.cache_arc(cache)?;
        self.touch_active_request()?;
        let batch = active_request_redis_batch(&cached)
            .ok_or_else(|| invalid("a batch step needs a request scope"))?;
        let index = self.record_batch(&batch, "request", cache);
        let handle = batch.mget(keys);
        self.handles.push(Some(Handle::Mget(handle)));
        self.steps.push(json!({
            "kind": "batch",
            "cache": cache,
            "batch": index,
        }));
        Ok(())
    }

    fn declare_post_call(
        &mut self,
        target: Target,
        call: &PostCall,
    ) -> Result<(), Box<HelperError>> {
        let cached = self.cache_arc(call.cache)?;
        let batch = match target {
            Target::Stored(request) => self.request_arc(request)?.post_call(&cached),
            Target::Active => {
                self.touch_active_request()?;
                active_post_call_redis_batch(&cached)
                    .ok_or_else(|| invalid("a post-call step needs a request scope"))?
            }
        };
        self.push_post_call(&batch, call);
        Ok(())
    }

    fn push_post_call(&mut self, batch: &Arc<RedisBatch>, call: &PostCall) {
        let index = self.record_batch(batch, "post_call", call.cache);
        let handle = batch.increment(&call.key, call.value, call.ttl);
        self.handles.push(Some(Handle::Increment(handle)));
        self.steps.push(json!({
            "kind": "post_call",
            "cache": call.cache,
            "batch": index,
        }));
    }

    async fn settle(&mut self, index: usize) -> Result<(), Box<HelperError>> {
        let taken = self
            .handles
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| invalid(format!("no unsettled handle {index}")))?;
        let outcome = settle_handle(taken, index).await;
        self.steps.push(outcome);
        Ok(())
    }

    async fn flush_post_call(&mut self) -> Result<(), Box<HelperError>> {
        flush_post_call_redis_batches().await;
        self.steps.push(json!({"kind": "flush_post_call"}));
        Ok(())
    }

    async fn flush_all(&mut self) -> Result<(), Box<HelperError>> {
        let request = active_request_redis_batches()
            .ok_or_else(|| invalid("a flush_all step needs a request scope"))?;
        request.flush_all().await;
        self.steps.push(json!({"kind": "flush_all"}));
        Ok(())
    }

    async fn pause(&mut self, millis: u64) -> Result<(), Box<HelperError>> {
        sleep(Duration::from_millis(millis)).await;
        self.steps.push(json!({"kind": "sleep"}));
        Ok(())
    }

    async fn nested(&mut self, cache: usize) -> Result<(), Box<HelperError>> {
        let cached = self.cache_arc(cache)?;
        let outer = Self::active_request()?;
        let request_before = self.record_request(&outer);
        let outer_batch = active_request_redis_batch(&cached)
            .ok_or_else(|| invalid("a nested step needs a request scope"))?;
        let batch_before = self.record_batch(&outer_batch, "request", cache);
        let deadline = self.deadline;
        let inside = request_redis_batch_scope(deadline, async {
            let request = active_request_redis_batches()
                .ok_or_else(|| invalid("the nested scope has no request"))?;
            let batch = active_request_redis_batch(&cached)
                .ok_or_else(|| invalid("the nested scope has no batch"))?;
            Ok::<_, Box<HelperError>>((request, batch))
        })
        .await;
        let (inside_request, inside_batch) = inside.1?;
        let request_inside = self.record_request(&inside_request);
        let batch_inside = self.record_batch(&inside_batch, "request", cache);
        let after = Self::active_request()?;
        let request_after = self.record_request(&after);
        let after_batch = active_request_redis_batch(&cached)
            .ok_or_else(|| invalid("the outer scope lost its batch"))?;
        let batch_after = self.record_batch(&after_batch, "request", cache);
        self.steps.push(json!({
            "kind": "nested",
            "cache": cache,
            "request_before": request_before,
            "batch_before": batch_before,
            "request_inside": request_inside,
            "batch_inside": batch_inside,
            "request_after": request_after,
            "batch_after": batch_after,
        }));
        Ok(())
    }

    async fn drain(&mut self) -> Result<(), Box<HelperError>> {
        drain_post_call_redis_batches().await;
        self.steps.push(json!({"kind": "drain"}));
        Ok(())
    }

    async fn create_request(&mut self) -> Result<(), Box<HelperError>> {
        let deadline = self.deadline;
        let (request, ()) = request_redis_batch_scope(deadline, async {}).await;
        let index = self.record_request(&request);
        self.steps
            .push(json!({"kind": "create_request", "request": index}));
        Ok(())
    }

    fn prefetch(&mut self, action: Prefetch) -> Result<(), Box<HelperError>> {
        let request = Self::active_request()?;
        let entry = match action {
            Prefetch::Arm { slot, value } => {
                drop(request.arm_prefetch(&slot, Box::new(value)));
                json!({
                    "kind": "prefetch_arm",
                    "slot": slot,
                    "armed": request.prefetch_armed(&slot),
                })
            }
            Prefetch::Take { slot } => {
                let taken = request
                    .take_prefetch(&slot)
                    .and_then(|value| value.downcast::<String>().ok())
                    .map(|value| *value);
                json!({
                    "kind": "prefetch_take",
                    "slot": slot,
                    "taken": taken,
                    "armed": request.prefetch_armed(&slot),
                })
            }
            Prefetch::Armed { slot } => json!({
                "kind": "prefetch_armed",
                "slot": slot,
                "armed": request.prefetch_armed(&slot),
            }),
        };
        self.steps.push(entry);
        Ok(())
    }

    fn touch_active_request(&mut self) -> Result<(), Box<HelperError>> {
        let request = Self::active_request()?;
        self.record_request(&request);
        Ok(())
    }

    fn active_request() -> Result<Arc<RequestRedisBatches>, Box<HelperError>> {
        active_request_redis_batches()
            .ok_or_else(|| invalid("the step needs an active request_redis_batch_scope"))
    }

    fn cache_arc(&self, index: usize) -> Result<Arc<RedisCache>, Box<HelperError>> {
        self.caches
            .get(index)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown cache {index}")))
    }

    fn request_arc(&self, index: usize) -> Result<Arc<RequestRedisBatches>, Box<HelperError>> {
        self.requests
            .get(index)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown request {index}")))
    }

    fn record_request(&mut self, request: &Arc<RequestRedisBatches>) -> usize {
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

    fn record_batch(&mut self, batch: &Arc<RedisBatch>, kind: &'static str, cache: usize) -> usize {
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

    fn snapshot(&self) -> Json {
        json!({
            "batches": self.batches_json(),
            "requests": self.requests.len(),
            "commands": self.commands_json(),
        })
    }

    fn batches_json(&self) -> Vec<Json> {
        self.batches
            .iter()
            .map(|entry| {
                json!({
                    "kind": entry.kind,
                    "caches": entry.caches,
                    "pending": entry.batch.pending(),
                    "flushes": entry.batch.flushes(),
                })
            })
            .collect()
    }

    fn commands_json(&self) -> BTreeMap<&str, Vec<String>> {
        self.servers
            .iter()
            .map(|(setting, server)| {
                let names = server
                    .command_names()
                    .into_iter()
                    .filter(|name| BATCH_COMMANDS.contains(&name.as_str()))
                    .collect();
                (setting.as_str(), names)
            })
            .collect()
    }

    fn report(self) -> Json {
        let batches = self.batches_json();
        let commands = self.commands_json();
        let requests = self.requests.len();
        json!({
            "steps": self.steps,
            "batches": batches,
            "requests": requests,
            "commands": commands,
        })
    }
}

async fn settle_handle(handle: Handle, index: usize) -> Json {
    match handle {
        Handle::Mget(result) => match result.await {
            Ok(values) => json!({
                "kind": "await",
                "handle": index,
                "ok": true,
                "value": presence(&values),
            }),
            Err(_error) => json!({"kind": "await", "handle": index, "ok": false}),
        },
        Handle::Increment(result) => match result.await {
            Ok(value) => json!({"kind": "await", "handle": index, "ok": true, "value": value}),
            Err(_error) => json!({"kind": "await", "handle": index, "ok": false}),
        },
    }
}

fn presence(values: &MgetValues) -> Json {
    values
        .iter()
        .map(|(key, value)| (key.clone(), Json::Bool(value.is_some())))
        .collect()
}
