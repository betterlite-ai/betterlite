use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::delivery::{BatchCancelled, BatchDelivery};

pub const BATCH_SIZE_VARIABLE: &str = "DEFAULT_BATCH_SIZE";
pub const FLUSH_INTERVAL_VARIABLE: &str = "DEFAULT_FLUSH_INTERVAL_SECONDS";
pub const DEFAULT_BATCH_SIZE: usize = 512;
pub const DEFAULT_FLUSH_INTERVAL_SECONDS: u64 = 5;
pub const DEFAULT_MAX_QUEUE_SIZE: usize = 50_000;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error(transparent)]
    Environment(#[from] betterlite_env::EnvError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchSettings {
    pub batch_size: usize,
    pub flush_interval: Duration,
    pub max_queue_size: usize,
}

impl Default for BatchSettings {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            flush_interval: Duration::from_secs(DEFAULT_FLUSH_INTERVAL_SECONDS),
            max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
        }
    }
}

impl BatchSettings {
    pub fn from_environment(environment: &BTreeMap<String, String>) -> Result<Self, SettingsError> {
        let batch_size = environment
            .get(BATCH_SIZE_VARIABLE)
            .map(|text| betterlite_env::number::<usize>(BATCH_SIZE_VARIABLE, text))
            .transpose()?
            .flatten()
            .unwrap_or(DEFAULT_BATCH_SIZE);
        let interval_seconds = environment
            .get(FLUSH_INTERVAL_VARIABLE)
            .map(|text| betterlite_env::number::<u64>(FLUSH_INTERVAL_VARIABLE, text))
            .transpose()?
            .flatten()
            .unwrap_or(DEFAULT_FLUSH_INTERVAL_SECONDS);
        Ok(Self {
            batch_size,
            flush_interval: Duration::from_secs(interval_seconds),
            max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushReport {
    pub batch: usize,
    pub delivered: usize,
    pub requeued: usize,
    pub dropped_oldest: usize,
    pub cancelled: bool,
}

#[derive(Debug)]
pub struct BatchQueue<E> {
    pub settings: BatchSettings,
    pub preserve_during_flush: bool,
    pub queue: Arc<Mutex<VecDeque<E>>>,
}

impl<E> Clone for BatchQueue<E> {
    fn clone(&self) -> Self {
        Self {
            settings: self.settings,
            preserve_during_flush: self.preserve_during_flush,
            queue: Arc::clone(&self.queue),
        }
    }
}

impl<E> BatchQueue<E> {
    #[must_use]
    pub fn new(settings: BatchSettings) -> Self {
        Self {
            settings,
            preserve_during_flush: false,
            queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    #[must_use]
    pub fn enqueue(&self, event: E) -> usize {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        queue.push_back(event);
        queue.len()
    }

    #[must_use]
    pub fn pending(&self) -> Vec<E>
    where
        E: Clone,
    {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    pub async fn flush<D>(&self, delivery: &D) -> FlushReport
    where
        E: Send,
        D: BatchDelivery<E> + Sync,
    {
        let batch: Vec<E> = {
            let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            queue.drain(..).collect()
        };
        let batch_len = batch.len();
        if batch_len == 0 {
            return FlushReport {
                batch: 0,
                delivered: 0,
                requeued: 0,
                dropped_oldest: 0,
                cancelled: false,
            };
        }
        let outcome = delivery.deliver(&batch).await;
        let cancelled = outcome.is_err();
        let undelivered = match outcome {
            Ok(undelivered) | Err(BatchCancelled { undelivered }) => undelivered,
        };
        let (requeued, dropped_oldest) = self.requeue(undelivered);
        if dropped_oldest > 0 {
            tracing::warn!(
                max_queue_size = self.settings.max_queue_size,
                dropped = dropped_oldest,
                "batch queue exceeded max_queue_size; dropped the oldest events"
            );
        }
        FlushReport {
            batch: batch_len,
            delivered: batch_len.saturating_sub(requeued),
            requeued,
            dropped_oldest,
            cancelled,
        }
    }

    fn requeue(&self, undelivered: Vec<E>) -> (usize, usize) {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        // A fully delivered flush still discards events that arrived while it ran, unless the caller keeps them
        if undelivered.is_empty() && !self.preserve_during_flush {
            queue.clear();
        }
        let requeued = undelivered.len();
        for event in undelivered.into_iter().rev() {
            queue.push_front(event);
        }
        let dropped_oldest = queue.len().saturating_sub(self.settings.max_queue_size);
        queue.drain(..dropped_oldest);
        drop(queue);
        (requeued, dropped_oldest)
    }
}

mod tests;
