use std::sync::Arc;

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::rate_limit_window_store::CounterScript;

pub trait ReleaseSettledHook: Send + Sync {
    fn settled(&self, failure: Option<&BatchError>);
}

pub trait ReleaseBatch: Send + Sync {
    fn register_release(
        &self,
        script: Arc<dyn CounterScript>,
        keys: &[String],
        args: &[RedisScalar],
        hook: Arc<dyn ReleaseSettledHook>,
    );
}

// None when the caller holds no post-call Redis batch, the batch is None LiteLLM guards on before deferring
pub trait ReleaseBatchSource: Send + Sync {
    fn active_release_batch(&self) -> Option<Arc<dyn ReleaseBatch>>;
}
