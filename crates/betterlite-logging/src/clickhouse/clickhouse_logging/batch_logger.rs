use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::batch::logger::{BatchQueue, BatchSettings};
use crate::trace_types::SpendLogRecord;

use super::settings::{ClickHouseSettings, ClickHouseSettingsError};
use super::storage::ClickHouseStorage;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClickHouseStats {
    pub rows_written: u64,
    pub rows_dropped: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Inserted {
    Delivered,
    Requeued,
    Dropped,
}

struct Inner<Storage> {
    storage: Storage,
    table: &'static str,
    settings: ClickHouseSettings,
    batch: BatchQueue<SpendLogRecord>,
    attempts: AtomicUsize,
    rows_written: AtomicU64,
    rows_dropped: AtomicU64,
}

pub struct ClickHouseBatchLogger<Storage: ClickHouseStorage> {
    inner: Arc<Inner<Storage>>,
    tasks: TaskTracker,
    shutdown: CancellationToken,
    started: AtomicBool,
    periodic_task: Mutex<Option<oneshot::Receiver<()>>>,
}

impl<Storage: ClickHouseStorage> ClickHouseBatchLogger<Storage> {
    pub fn new(
        storage: Storage,
        table: &'static str,
        settings: ClickHouseSettings,
        tasks: TaskTracker,
    ) -> Self {
        let batch_settings = BatchSettings {
            batch_size: settings.batch_size,
            flush_interval: settings.flush_interval,
            // B256 drops the oldest above this cap; ClickHouse's buffer cap is only the is_full hint
            max_queue_size: usize::MAX,
        };
        let mut batch = BatchQueue::new(batch_settings);
        // rows enqueued while an insert runs wait for the next flush instead of being cleared
        batch.preserve_during_flush = true;
        Self {
            inner: Arc::new(Inner {
                storage,
                table,
                settings,
                batch,
                attempts: AtomicUsize::new(0),
                rows_written: AtomicU64::new(0),
                rows_dropped: AtomicU64::new(0),
            }),
            tasks,
            shutdown: CancellationToken::new(),
            started: AtomicBool::new(false),
            periodic_task: Mutex::new(None),
        }
    }

    pub fn from_environment<Factory>(
        environment: &dyn EnvironmentLookup,
        table: &'static str,
        tasks: TaskTracker,
        open_storage: Factory,
    ) -> Result<Self, ClickHouseSettingsError>
    where
        Factory: FnOnce(&ClickHouseSettings) -> Storage,
    {
        let settings = ClickHouseSettings::from_environment(environment)?;
        let storage = open_storage(&settings);
        Ok(Self::new(storage, table, settings, tasks))
    }

    pub fn start(&self) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let (finished, periodic_task) = oneshot::channel();
        *self
            .periodic_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(periodic_task);
        let inner = Arc::clone(&self.inner);
        let shutdown = self.shutdown.clone();
        let _spawned = self.tasks.spawn(async move {
            inner.periodic_flush(&shutdown).await;
            let _finished = finished.send(());
        });
    }

    pub fn enqueue<Rows>(&self, rows: Rows)
    where
        Rows: IntoIterator<Item = SpendLogRecord>,
    {
        self.start();
        for row in rows {
            let _enqueued = self.inner.batch.enqueue(row);
        }
        if self.inner.buffer_depth() >= self.inner.settings.batch_size {
            let inner = Arc::clone(&self.inner);
            let _spawned = self.tasks.spawn(async move {
                inner.batch.flush(&InsertDelivery { inner: &inner }).await;
            });
        }
    }

    pub async fn flush_queue(&self) {
        self.inner
            .batch
            .flush(&InsertDelivery { inner: &self.inner })
            .await;
    }

    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        let periodic = self
            .periodic_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(finished) = periodic {
            let _finished = finished.await;
        }
        while self.inner.buffer_depth() > 0 {
            self.inner
                .batch
                .flush(&InsertDelivery { inner: &self.inner })
                .await;
        }
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        // LiteLLM only exposes this as a backpressure hint; enqueue itself never rejects a row
        self.inner.buffer_depth() >= self.inner.settings.max_buffered_rows
    }

    #[must_use]
    pub fn stats(&self) -> ClickHouseStats {
        ClickHouseStats {
            rows_written: self.inner.rows_written.load(Ordering::SeqCst),
            rows_dropped: self.inner.rows_dropped.load(Ordering::SeqCst),
        }
    }
}

impl<Storage: ClickHouseStorage> Drop for ClickHouseBatchLogger<Storage> {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

impl<Storage: ClickHouseStorage> Inner<Storage> {
    fn buffer_depth(&self) -> usize {
        self.batch
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    async fn periodic_flush(&self, shutdown: &CancellationToken) {
        loop {
            if timeout(self.settings.flush_interval, shutdown.cancelled())
                .await
                .is_ok()
            {
                return;
            }
            self.batch.flush(&InsertDelivery { inner: self }).await;
        }
    }

    async fn insert(&self, batch: &[SpendLogRecord]) -> Inserted {
        match self.storage.insert_rows(self.table, batch).await {
            Ok(()) => {
                let rows = u64::try_from(batch.len()).unwrap_or(u64::MAX);
                self.rows_written.fetch_add(rows, Ordering::SeqCst);
                self.attempts.store(0, Ordering::SeqCst);
                Inserted::Delivered
            }
            Err(error) => {
                let failures = self
                    .attempts
                    .fetch_add(1, Ordering::SeqCst)
                    .saturating_add(1);
                if failures >= self.settings.max_retries {
                    let rows = u64::try_from(batch.len()).unwrap_or(u64::MAX);
                    self.rows_dropped.fetch_add(rows, Ordering::SeqCst);
                    self.attempts.store(0, Ordering::SeqCst);
                    tracing::error!(
                        rows = batch.len(),
                        table = self.table,
                        %error,
                        "ClickHouse: dropped rows after repeated insert failures"
                    );
                    Inserted::Dropped
                } else {
                    tracing::warn!(
                        table = self.table,
                        %error,
                        "ClickHouse: insert failed, will retry"
                    );
                    Inserted::Requeued
                }
            }
        }
    }
}

struct InsertDelivery<'logger, Storage: ClickHouseStorage> {
    inner: &'logger Inner<Storage>,
}

impl<Storage: ClickHouseStorage> BatchDelivery<SpendLogRecord> for InsertDelivery<'_, Storage> {
    async fn deliver(
        &self,
        batch: &[SpendLogRecord],
        settled: &AtomicUsize,
    ) -> Result<Vec<SpendLogRecord>, BatchCancelled<SpendLogRecord>> {
        let mut chunks = batch.chunks(self.inner.settings.batch_size.max(1));
        while let Some(chunk) = chunks.next() {
            let inserted = self.inner.insert(chunk).await;
            if !matches!(inserted, Inserted::Requeued) {
                settled.fetch_add(chunk.len(), Ordering::SeqCst);
            }
            match inserted {
                Inserted::Delivered => {}
                Inserted::Dropped => return Ok(chunks.flatten().cloned().collect()),
                Inserted::Requeued => {
                    return Ok(chunk.iter().chain(chunks.flatten()).cloned().collect());
                }
            }
        }
        Ok(Vec::new())
    }
}

mod tests;
