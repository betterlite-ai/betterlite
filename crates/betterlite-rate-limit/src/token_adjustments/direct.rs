use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use crate::rate_limit_window_store::hash_tag_group::group_keys_by_hash_tag;

use super::deferred::defer_token_increment_script;
use super::scripts::token_increment_args;
use super::store::{TokenAdjustmentError, TokenAdjustments};

pub async fn increment_tokens_keeping_ttl(
    adjustments: &TokenAdjustments,
    operations: &[IncrementOperation],
) {
    if operations.is_empty() {
        return;
    }
    if adjustments.token_increment_script.is_none() {
        tracing::debug!("the ttl preservation script is not available, using the regular pipeline");
        adjustments.usage.increment_pipeline(operations).await;
        return;
    }
    match execute_token_increment_script(adjustments, operations).await {
        Ok(()) => {
            tracing::debug!(count = operations.len(), "the ttl-preserving increment ran");
        }
        Err(error) => {
            tracing::warn!(%error, "ttl preservation failed, falling back to the regular pipeline");
            adjustments.usage.increment_pipeline(operations).await;
        }
    }
}

pub async fn execute_token_increment_script(
    adjustments: &TokenAdjustments,
    operations: &[IncrementOperation],
) -> Result<(), TokenAdjustmentError> {
    let Some(script) = adjustments.token_increment_script.as_deref() else {
        return Ok(());
    };
    let keys: Vec<String> = operations
        .iter()
        .map(|operation| operation.key.clone())
        .collect();
    for (_tag, group_keys) in group_keys_by_hash_tag(&adjustments.windows, &keys) {
        let group_operations: Vec<IncrementOperation> = operations
            .iter()
            .filter(|operation| group_keys.contains(&operation.key))
            .cloned()
            .collect();
        for operation in &group_operations {
            tracing::debug!(
                key = %operation.key,
                increment = operation.increment_value,
                ttl = ?operation.ttl,
                "executing a ttl-preserving increment"
            );
        }
        let args = token_increment_args(&group_operations);
        if defer_token_increment_script(adjustments, &group_keys, &args, &group_operations) {
            continue;
        }
        let _reply = script.run(&group_keys, &args).await?;
    }
    Ok(())
}
