use std::num::ParseFloatError;
use std::str::Utf8Error;
use std::sync::Arc;

use betterlite_redis::batch::result::SettledHook;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;
use num_traits::ToPrimitive as _;

use crate::limiter_records::ScriptReply;
use crate::rate_limit_window_store::{CounterScript, WindowStore};

// f64 holds every whole number to 2^53 exactly, which is where a token count stops being countable
const MAX_EXACT_TOKEN_AMOUNT: u64 = 0x0020_0000_0000_0000;

pub struct TokenAdjustments {
    pub windows: WindowStore,
    pub usage: Arc<dyn UsageIncrements>,
    pub post_call_batch: Option<Arc<dyn PostCallScriptBatch>>,
    pub token_increment_script: Option<Arc<dyn CounterScript>>,
    pub guarded_token_script: Option<Arc<dyn CounterScript>>,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenAdjustmentError {
    #[error("the token increment script run failed")]
    Script(#[from] RedisCountersError),
    #[error("the cached counter text is not a number")]
    CounterText(#[source] ParseFloatError),
    #[error("the cached counter bytes are not UTF-8")]
    CounterBytes(#[source] Utf8Error),
    #[error("the token amount {value} has no exact float form")]
    TokenAmount { value: i64 },
}

// the dual-cache writes LiteLLM runs for a token adjustment; a cache failure falls back inside the cache
#[async_trait::async_trait]
pub trait UsageIncrements: Send + Sync {
    async fn increment(&self, operation: &IncrementOperation);
    async fn increment_pipeline(&self, operations: &[IncrementOperation]);
    async fn set_local_counter(&self, key: &str, value: f64, ttl: Option<i64>);
}

// the request's post-call batch, which runs a registered script and its hook when the batch settles
pub trait PostCallScriptBatch: Send + Sync {
    fn script_on_settled(
        &self,
        script: Arc<dyn CounterScript>,
        keys: &[String],
        args: &[RedisScalar],
        hook: SettledHook<ScriptReply>,
    );
}

pub fn token_float(value: i64) -> Result<f64, TokenAdjustmentError> {
    if value.unsigned_abs() > MAX_EXACT_TOKEN_AMOUNT {
        return Err(TokenAdjustmentError::TokenAmount { value });
    }
    value
        .to_f64()
        .ok_or(TokenAdjustmentError::TokenAmount { value })
}
