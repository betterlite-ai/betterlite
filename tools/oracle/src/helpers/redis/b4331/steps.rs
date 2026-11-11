use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::batch::MgetValues;
use betterlite_redis::request_batch::lock;
use betterlite_redis::request_batch::registry::drain_post_call_redis_batches;
use betterlite_redis::request_batch::scope::{
    active_post_call_redis_batch, active_request_redis_batch, flush_post_call_redis_batches,
    request_redis_batch_scope,
};
use serde_json::{Value as Json, json};
use tokio::time::sleep;

use super::input::{PostCall, Step};
use super::scenario::{Handle, Prefetch, Scenario, Target, invalid};
use crate::helpers::HelperError;

impl Scenario<'_> {
    pub(crate) async fn step(&mut self, step: &Step) -> Result<(), Box<HelperError>> {
        match step {
            Step::Batch { cache, keys } => self.declare_batch(*cache, keys),
            Step::PostCall { call } => self.declare_post_call(Target::Active, call),
            Step::PostCallOn { request, call } => {
                self.declare_post_call(Target::Stored(*request), call)
            }
            Step::Await { handle } => self.settle(*handle).await,
            Step::FlushPostCall => flush_post_call().await,
            Step::Sleep { millis } => pause(*millis).await,
            Step::Nested { cache } => self.nested(*cache).await,
            Step::Drain => drain().await,
            Step::CreateRequest => self.create_request().await,
            Step::PrefetchArm { slot, value } => self.prefetch(Prefetch::Arm {
                slot: slot.clone(),
                value: value.clone(),
            }),
            Step::PrefetchTake { slot } => self.prefetch(Prefetch::Take { slot: slot.clone() }),
            Step::Snapshot => {
                self.snapshots.push(self.state());
                Ok(())
            }
        }
    }

    fn declare_batch(&mut self, cache: usize, keys: &[String]) -> Result<(), Box<HelperError>> {
        let cached = self.cache_arc(cache)?;
        self.touch_active_request()?;
        let batch = active_request_redis_batch(&cached)
            .ok_or_else(|| invalid("a batch step needs a request scope"))?;
        self.record_batch(&batch, "request", cache);
        let handle = batch.mget(keys);
        self.handles.push(Some(Handle::Mget(handle)));
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
        self.record_batch(&batch, "post_call", call.cache);
        let handle = batch.increment(&call.key, call.value, call.ttl);
        self.handles.push(Some(Handle::Increment(handle)));
        Ok(())
    }

    async fn settle(&mut self, index: usize) -> Result<(), Box<HelperError>> {
        let taken = self
            .handles
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| invalid(format!("no unsettled handle {index}")))?;
        let outcome = settle_handle(taken, index).await;
        self.awaited.push(outcome);
        Ok(())
    }

    async fn nested(&mut self, cache: usize) -> Result<(), Box<HelperError>> {
        let cached = self.cache_arc(cache)?;
        let before = Self::active_request()?;
        self.record_request(&before);
        let before_batch = active_request_redis_batch(&cached)
            .ok_or_else(|| invalid("a nested step needs a request scope"))?;
        self.record_batch(&before_batch, "request", cache);
        let deadline = self.deadline;
        let inside = request_redis_batch_scope(deadline, async {
            match (Self::active_request(), active_request_redis_batch(&cached)) {
                (Ok(request), Some(batch)) => Ok((request, batch)),
                _ => Err(invalid("the nested scope lost the outer request or batch")),
            }
        })
        .await
        .1?;
        let after = Self::active_request()?;
        let after_batch = active_request_redis_batch(&cached)
            .ok_or_else(|| invalid("the outer scope lost its batch"))?;
        self.record_request(&inside.0);
        self.record_request(&after);
        let entry = json!({
            "cache": cache,
            "request_shared": Arc::ptr_eq(&inside.0, &before) && Arc::ptr_eq(&after, &before),
            "batch_shared": Arc::ptr_eq(&inside.1, &before_batch) && Arc::ptr_eq(&after_batch, &before_batch),
        });
        self.nested.push(entry);
        Ok(())
    }

    async fn create_request(&mut self) -> Result<(), Box<HelperError>> {
        let deadline = self.deadline;
        let (request, ()) = request_redis_batch_scope(deadline, async {}).await;
        self.record_request(&request);
        Ok(())
    }

    fn prefetch(&mut self, action: Prefetch) -> Result<(), Box<HelperError>> {
        let request = Self::active_request()?;
        let (kind, slot, taken) = match action {
            Prefetch::Arm { slot, value } => {
                let _replaced = lock(&request.prefetched).insert(slot.clone(), Box::new(value));
                ("arm", slot, Json::Null)
            }
            Prefetch::Take { slot } => {
                let taken = lock(&request.prefetched)
                    .remove(&slot)
                    .and_then(|value| value.downcast::<String>().ok())
                    .map(|value| *value);
                ("take", slot, taken.map_or(Json::Null, Json::String))
            }
        };
        let armed = lock(&request.prefetched).contains_key(&slot);
        self.prefetch
            .push(json!({"action": kind, "slot": slot, "taken": taken, "armed": armed}));
        Ok(())
    }
}

async fn flush_post_call() -> Result<(), Box<HelperError>> {
    flush_post_call_redis_batches().await;
    Ok(())
}

async fn pause(millis: u64) -> Result<(), Box<HelperError>> {
    sleep(Duration::from_millis(millis)).await;
    Ok(())
}

async fn drain() -> Result<(), Box<HelperError>> {
    drain_post_call_redis_batches().await;
    Ok(())
}

async fn settle_handle(handle: Handle, index: usize) -> Json {
    match handle {
        Handle::Mget(result) => match result.await {
            Ok(values) => json!({
                "handle": index,
                "ok": true,
                "value": presence(&values),
            }),
            Err(_error) => json!({"handle": index, "ok": false}),
        },
        Handle::Increment(result) => match result.await {
            Ok(value) => json!({"handle": index, "ok": true, "value": value}),
            Err(_error) => json!({"handle": index, "ok": false}),
        },
    }
}

fn presence(values: &MgetValues) -> Json {
    values
        .iter()
        .map(|(key, value)| (key.clone(), Json::Bool(value.is_some())))
        .collect()
}
