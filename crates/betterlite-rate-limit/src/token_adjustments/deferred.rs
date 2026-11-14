use std::sync::Arc;

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::{Settled, SettledFollowUp, SettledHook};
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::limiter_records::ScriptReply;

use super::store::TokenAdjustments;

pub(super) fn defer_token_increment_script(
    adjustments: &TokenAdjustments,
    keys: &[String],
    args: &[RedisScalar],
    operations: &[IncrementOperation],
) -> bool {
    let (Some(script), Some(batch)) = (
        adjustments.token_increment_script.as_ref(),
        adjustments.post_call_batch.as_ref(),
    ) else {
        return false;
    };
    let usage = Arc::clone(&adjustments.usage);
    let fallback_operations = operations.to_vec();
    let hook: SettledHook<ScriptReply> = Arc::new(
        move |settled: &Settled<ScriptReply>| -> Option<SettledFollowUp> {
            let Err(failure) = settled else {
                return None;
            };
            let error: &BatchError = failure.as_ref();
            if matches!(error, BatchError::Cancelled) {
                return None;
            }
            tracing::warn!(%error, "ttl preservation failed, falling back to the regular pipeline");
            let usage = Arc::clone(&usage);
            let operations = fallback_operations.clone();
            Some(Box::pin(async move {
                usage.increment_pipeline(&operations).await;
            }))
        },
    );
    batch.script_on_settled(Arc::clone(script), keys, args, hook);
    true
}
