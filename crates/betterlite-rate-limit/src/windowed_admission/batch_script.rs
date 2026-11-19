use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::limiter_records::{CacheCounterValue, counter_values_from_reply};
use crate::rate_limit_window_store::batch_rate_limiter_script::{
    run_batch_rate_limiter_script, window_script_args,
};
use crate::rate_limit_window_store::hash_tag_group::group_keys_by_hash_tag;
use crate::rate_limit_window_store::in_memory_window::in_memory_cache_sliding_window;
use crate::rate_limit_window_store::pipeline_scripts::pipeline_scripts;
use crate::rate_limit_window_store::{CounterValues, ScriptCall, ScriptHandle};
use crate::settlement_and_rollback::AtomicSettlement;
// ast-grep-ignore: no-long-names
use crate::settlement_and_rollback::refunds::counter_refunds_from_batch_values;

use super::{WindowedAdmission, WindowedAdmissionError};

impl WindowedAdmission {
    fn settlement(&self) -> AtomicSettlement<'_> {
        AtomicSettlement {
            windows: &self.windows,
            clock: self.clock.as_ref(),
            script: None,
            refund_script: self.refund_script.clone(),
            fail_closed: self.fail_closed,
            default_window_size: self.window_size,
        }
    }

    // ast-grep-ignore: no-long-names
    pub async fn execute_redis_batch_rate_limiter_script(
        &self,
        keys_to_fetch: &[String],
        now_int: i64,
    ) -> Result<Vec<CacheCounterValue>, WindowedAdmissionError> {
        let Some(script) = self.windows.rate_limiter_script.as_ref() else {
            return Ok(Vec::new());
        };
        let groups = group_keys_by_hash_tag(&self.windows, keys_to_fetch);
        let args = window_script_args(now_int, self.window_size);
        let calls = group_calls(&groups, &args);
        let mut handles = pipeline_scripts(&self.windows, script, &calls);
        let mut values = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            let handle = handles.get_mut(index).and_then(Option::take);
            let outcome = match handle {
                Some(handle) => match handle.await {
                    Ok(reply) => {
                        counter_values_from_reply(&reply).map_err(WindowedAdmissionError::from)
                    }
                    Err(error) => Err(WindowedAdmissionError::Batch(error)),
                },
                None => run_batch_rate_limiter_script(&self.windows, &group.1, &args)
                    .await
                    .map_err(WindowedAdmissionError::from),
            };
            match outcome {
                Ok(group_values) => values.extend(group_values),
                Err(error) => {
                    self.refund_on_failure(&groups, index, &values, &mut handles)
                        .await?;
                    let fallback = self.fallback_group_values(&error, group, now_int).await?;
                    values.extend(fallback.into_iter().flatten());
                }
            }
        }
        Ok(values)
    }

    pub async fn refund_later_pipelined_groups(
        &self,
        groups: &[(String, Vec<String>)],
        handles: &mut [Option<ScriptHandle>],
    ) -> Result<(), WindowedAdmissionError> {
        for (group, slot) in groups.iter().zip(handles.iter_mut()) {
            let Some(handle) = slot.take() else {
                continue;
            };
            let Ok(reply) = handle.await else {
                continue;
            };
            let Ok(group_values) = counter_values_from_reply(&reply) else {
                continue;
            };
            let applied: CounterValues = group_values.iter().cloned().map(Some).collect();
            // ast-grep-ignore: no-long-names
            let refunds = counter_refunds_from_batch_values(&group.1, &applied)?;
            self.settlement().refund_counter_increments(&refunds).await;
        }
        Ok(())
    }

    async fn refund_on_failure(
        &self,
        groups: &[(String, Vec<String>)],
        index: usize,
        values: &[CacheCounterValue],
        handles: &mut [Option<ScriptHandle>],
    ) -> Result<(), WindowedAdmissionError> {
        if !self.fail_closed.enabled {
            return Ok(());
        }
        let applied_keys: Vec<String> = groups
            .iter()
            .take(index)
            .flat_map(|(_tag, group_keys)| group_keys.iter().cloned())
            .collect();
        let applied: CounterValues = values.iter().cloned().map(Some).collect();
        // ast-grep-ignore: no-long-names
        let refunds = counter_refunds_from_batch_values(&applied_keys, &applied)?;
        self.settlement().refund_counter_increments(&refunds).await;
        let later = index.saturating_add(1);
        if let (Some(later_groups), Some(later_handles)) =
            (groups.get(later..), handles.get_mut(later..))
        {
            self.refund_later_pipelined_groups(later_groups, later_handles)
                .await?;
        }
        Ok(())
    }

    async fn fallback_group_values(
        &self,
        failure: &WindowedAdmissionError,
        group: &(String, Vec<String>),
        now_int: i64,
    ) -> Result<CounterValues, WindowedAdmissionError> {
        self.fail_closed
            .reject_if_unverifiable("batch_rate_limiter_script", failure)?;
        let hash_tag = &group.0;
        tracing::warn!(error = %failure, "Redis Lua script failed for hash tag {hash_tag}");
        let fallback =
            in_memory_cache_sliding_window(&self.windows, &group.1, now_int, self.window_size)
                .await?;
        Ok(fallback)
    }
}

fn group_calls(groups: &[(String, Vec<String>)], args: &[RedisScalar]) -> Vec<ScriptCall> {
    groups
        .iter()
        .map(|(_tag, group_keys)| (group_keys.clone(), args.to_vec()))
        .collect()
}

mod tests;
