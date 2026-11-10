use std::sync::Arc;

use redis::Value;

use super::delete::DeleteOperation;
use super::error::{BatchError, PipelineFailure};
use super::increment::IncrementOperation;
use super::mget::MgetOperation;
use super::replies::settle_pipeline;
use super::result::{SettledHook, SettledStatus};
use super::script::ScriptOperation;
use super::set::SetOperation;
use crate::redis_cache::RedisCache;
use crate::redis_live_client::LiveClientError;

pub(crate) enum Operation {
    Mget(MgetOperation),
    Script(ScriptOperation),
    Increment(IncrementOperation),
    Set(SetOperation),
    Delete(DeleteOperation),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Settlement {
    Done,
    Retry,
}

impl Operation {
    pub(crate) fn enqueue(
        &mut self,
        cache: &RedisCache,
        pipeline: &mut redis::Pipeline,
    ) -> Result<usize, PipelineFailure> {
        match self {
            Self::Mget(op) => Ok(op.enqueue(cache, pipeline)),
            Self::Script(op) => Ok(op.enqueue(cache, pipeline)),
            Self::Increment(op) => Ok(op.enqueue(cache, pipeline)),
            Self::Set(op) => op.enqueue(cache, pipeline),
            Self::Delete(op) => Ok(op.enqueue(cache, pipeline)),
        }
    }

    pub(crate) fn settle(&self, replies: Vec<Result<Value, LiveClientError>>) -> Settlement {
        match self {
            Self::Mget(op) => op.settle(replies),
            Self::Script(op) => op.settle(replies),
            Self::Increment(op) => op.settle(replies),
            Self::Set(op) => {
                settle_pipeline(replies, |_values| Ok(()), |result| op.slot.complete(result))
            }
            Self::Delete(op) => {
                settle_pipeline(replies, |_values| Ok(()), |result| op.slot.complete(result))
            }
        }
    }

    pub(crate) async fn settle_alone(&self, cache: &RedisCache) {
        match self {
            Self::Mget(op) => op.settle_alone(cache).await,
            Self::Script(op) => op.settle_alone(cache).await,
            Self::Increment(op) => op.settle_alone(cache).await,
            Self::Set(op) => op.settle_alone(cache).await,
            Self::Delete(op) => op.settle_alone(cache).await,
        }
    }

    pub(crate) fn settle_unresolved(&self, failure: Option<Arc<BatchError>>) {
        match self {
            Self::Mget(op) => op.slot.settle_unresolved(failure),
            Self::Script(op) => op.slot.settle_unresolved(failure),
            Self::Increment(op) => op.slot.settle_unresolved(failure),
            Self::Set(op) => op.slot.settle_unresolved(failure),
            Self::Delete(op) => op.slot.settle_unresolved(failure),
        }
    }

    fn hooks(&self) -> (Vec<SettledHook>, SettledStatus) {
        match self {
            Self::Mget(op) => (op.slot.take_hooks(), op.slot.status()),
            Self::Script(op) => (op.slot.take_hooks(), op.slot.status()),
            Self::Increment(op) => (op.slot.take_hooks(), op.slot.status()),
            Self::Set(op) => (op.slot.take_hooks(), op.slot.status()),
            Self::Delete(op) => (op.slot.take_hooks(), op.slot.status()),
        }
    }

    pub(crate) async fn run_hooks(&self) {
        let (hooks, status) = self.hooks();
        for hook in hooks {
            if let Some(follow_up) = hook(status) {
                follow_up.await;
            }
        }
    }

    pub(crate) fn run_hooks_now(&self) {
        let (hooks, status) = self.hooks();
        for hook in hooks {
            // a cancelled flush cannot await a follow-up; the hook ran and its future is dropped
            let _pending = hook(status);
        }
    }
}
