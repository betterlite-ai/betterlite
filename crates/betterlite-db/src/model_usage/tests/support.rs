use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rust_decimal::Decimal;

use crate::migration_ledger::lock::JitterSource;
use crate::model_usage::{
    ModelUsageFailure, ModelUsageFuture, ModelUsageKey, ModelUsageRollup, ModelUsageTransaction,
    ModelUsageWriter,
};
use crate::spend_queues::entity::SpendAmount;
use crate::tool_spend::RetryPause;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn key(
    date: &str,
    model_group: &str,
    model: &str,
    provider: &str,
    task_type: &str,
) -> ModelUsageKey {
    ModelUsageKey {
        date: date.to_owned(),
        model_group: model_group.to_owned(),
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        task_type: task_type.to_owned(),
    }
}

pub(super) fn transaction(
    row_key: ModelUsageKey,
    spend: Decimal,
    tokens: (i64, i64),
    successful: bool,
) -> ModelUsageTransaction {
    ModelUsageTransaction {
        key: row_key,
        spend: SpendAmount(spend),
        prompt_tokens: tokens.0,
        completion_tokens: tokens.1,
        successful,
    }
}

pub(super) fn rollup(entry: ModelUsageTransaction, counts: (i64, i64, i64)) -> ModelUsageRollup {
    ModelUsageRollup {
        key: entry.key,
        spend: entry.spend.0,
        prompt_tokens: entry.prompt_tokens,
        completion_tokens: entry.completion_tokens,
        request_count: counts.0,
        successful_requests: counts.1,
        failed_requests: counts.2,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum WriteFailure {
    #[error("the write never reached the database")]
    PreSend,
    #[error("the write's outcome is unknown")]
    AfterSend,
}

impl ModelUsageFailure for WriteFailure {
    fn is_pre_send(&self) -> bool {
        matches!(self, Self::PreSend)
    }
}

// A writer that records every batch it is handed and answers with the scripted failures in order.
pub(super) struct ScriptedWriter {
    batches: Mutex<Vec<Vec<ModelUsageRollup>>>,
    script: Mutex<VecDeque<WriteFailure>>,
}

impl ScriptedWriter {
    pub(super) fn new<Script>(script: Script) -> Self
    where
        Script: IntoIterator<Item = WriteFailure>,
    {
        Self {
            batches: Mutex::new(Vec::new()),
            script: Mutex::new(script.into_iter().collect()),
        }
    }

    pub(super) fn batches(&self) -> Vec<Vec<ModelUsageRollup>> {
        lock(&self.batches).clone()
    }
}

impl ModelUsageWriter for ScriptedWriter {
    type Error = WriteFailure;

    fn apply_rollups<'writer>(
        &'writer self,
        rollups: &'writer [ModelUsageRollup],
    ) -> ModelUsageFuture<'writer, Self::Error> {
        Box::pin(async move {
            lock(&self.batches).push(rollups.to_vec());
            let next = lock(&self.script).pop_front();
            match next {
                Some(failure) => Err(failure),
                None => Ok(()),
            }
        })
    }
}

#[derive(Default)]
pub(super) struct RecordingPause {
    delays: Mutex<Vec<Duration>>,
}

impl RecordingPause {
    pub(super) fn delays(&self) -> Vec<Duration> {
        lock(&self.delays).clone()
    }
}

impl RetryPause for RecordingPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        lock(&self.delays).push(delay);
        Box::pin(async {})
    }
}

pub(super) struct FixedJitter {
    pub(super) nanos: u64,
}

impl JitterSource for FixedJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        self.nanos.min(upper_exclusive_nanos.saturating_sub(1))
    }
}
