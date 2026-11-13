use std::mem;
use std::sync::Arc;
use std::time::Instant;

use redis::Value;
use tokio::runtime::Handle;

use super::RedisBatch;
use super::error::{BatchError, PipelineFailure};
use super::operation::{Operation, Settlement};
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_live_client::LiveClientError;
use crate::redis_live_client::pipeline::LivePipeline;

pub(crate) async fn flush(batch: &RedisBatch) {
    let flush_hooks: Vec<Arc<dyn Fn() + Send + Sync>> = match batch.flush_hooks.lock() {
        Ok(hooks) => hooks.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    for hook in flush_hooks {
        hook();
    }
    let pending = match batch.pending.lock() {
        Ok(mut pending) => mem::take(&mut *pending),
        Err(poisoned) => mem::take(&mut *poisoned.into_inner()),
    };
    if pending.is_empty() {
        return;
    }
    count_flush(batch);
    let mut draining = Draining {
        batch,
        ops: pending,
        done: false,
    };
    match batch.redis_cache.transport.family {
        RedisCacheFamily::Cluster => run_standalone(batch, &draining.ops).await,
        RedisCacheFamily::Standalone => run_pipeline(batch, &mut draining.ops).await,
    }
    draining.finish().await;
}

async fn run_standalone(batch: &RedisBatch, ops: &[Operation]) {
    for op in ops {
        op.settle_alone(&batch.redis_cache).await;
    }
}

async fn run_pipeline(batch: &RedisBatch, ops: &mut [Operation]) {
    let started = Instant::now();
    match execute_pipeline(batch, ops).await {
        Ok((widths, replies)) => {
            batch.redis_cache.hooks.success(&ServiceEvent {
                call_type: batch.name,
                duration: started.elapsed(),
                key: None,
            });
            settle_replies(batch, ops, widths, replies).await;
        }
        Err(failure) => {
            batch.redis_cache.hooks.failure(
                &ServiceEvent {
                    call_type: batch.name,
                    duration: started.elapsed(),
                    key: None,
                },
                &failure,
            );
            tracing::warn!(
                error = %failure,
                "{}: pipeline of {} ops failed",
                batch.name,
                ops.len()
            );
            let error = Arc::new(BatchError::Pipeline(failure));
            for op in ops.iter() {
                op.settle_unresolved(Some(Arc::clone(&error)));
            }
        }
    }
}

async fn execute_pipeline(
    batch: &RedisBatch,
    ops: &mut [Operation],
) -> Result<(Vec<usize>, Vec<Result<Value, LiveClientError>>), PipelineFailure> {
    let admission = batch.redis_cache.gate.enter(batch.name)?;
    match run_commands(batch, ops).await {
        Ok(replies) => {
            batch.redis_cache.gate.exit(admission);
            Ok(replies)
        }
        Err(failure) => {
            batch.redis_cache.gate.record_failure(&failure);
            Err(failure)
        }
    }
}

async fn run_commands(
    batch: &RedisBatch,
    ops: &mut [Operation],
) -> Result<(Vec<usize>, Vec<Result<Value, LiveClientError>>), PipelineFailure> {
    let client = batch.redis_cache.transport.async_client().await?;
    let mut pipeline = LivePipeline::new(client, false);
    let mut widths = Vec::with_capacity(ops.len());
    for op in ops.iter_mut() {
        widths.push(op.enqueue(&batch.redis_cache, &mut pipeline.commands)?);
    }
    let replies = pipeline.execute_settled().await?;
    Ok((widths, replies))
}

fn count_flush(batch: &RedisBatch) {
    let mut flushes = match batch.flushes.lock() {
        Ok(flushes) => flushes,
        Err(poisoned) => poisoned.into_inner(),
    };
    let next = flushes.saturating_add(1);
    *flushes = next;
}

async fn settle_replies(
    batch: &RedisBatch,
    ops: &[Operation],
    widths: Vec<usize>,
    replies: Vec<Result<Value, LiveClientError>>,
) {
    let mut remaining = replies.into_iter();
    let mut retries = Vec::new();
    for (op, width) in ops.iter().zip(widths) {
        let slice: Vec<Result<Value, LiveClientError>> = remaining.by_ref().take(width).collect();
        if op.settle(slice) == Settlement::Retry {
            retries.push(op);
        }
    }
    for op in retries {
        op.settle_alone(&batch.redis_cache).await;
    }
}

struct Draining<'batch> {
    batch: &'batch RedisBatch,
    ops: Vec<Operation>,
    done: bool,
}

impl Draining<'_> {
    async fn finish(mut self) {
        for op in &self.ops {
            op.settle_unresolved(None);
        }
        for call in self.ops.iter().flat_map(Operation::pending_hooks) {
            if let Some(follow_up) = call() {
                follow_up.await;
            }
        }
        self.done = true;
    }
}

impl Drop for Draining<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        for op in &self.ops {
            op.settle_unresolved(None);
        }
        let mut follow_ups = Vec::new();
        for op in &self.ops {
            follow_ups.extend(op.pending_hooks().into_iter().filter_map(|call| call()));
        }
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        for follow_up in follow_ups {
            self.batch.follow_ups.spawn_on(follow_up, &runtime);
        }
    }
}
