use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use indexmap::IndexMap;
use rand::RngExt as _;

use super::credentials::{ArgillaCredentials, DatasetError, resolve_dataset};
use super::delivery::{ArgillaDelivery, BatchMode};
use super::extraction::{ArgillaItem, CallbackKwargs, prepare_log_data};
use super::hooks::{DatasetHook, DatasetHookError};
use super::settings::{
    ArgillaSettings, PayloadField, SYNC_SAMPLING_RATE_VARIABLE, digit_sampling_rate,
};
use super::transport::Transport;
use crate::batch::delivery::BatchDelivery;
use crate::batch::logger::{BatchQueue, FlushReport};

pub trait RandomSource: Send + Sync {
    fn sample(&self) -> f64;
}

pub struct SeededRandom<R> {
    pub generator: Mutex<R>,
}

impl<R: rand::Rng + Send> RandomSource for SeededRandom<R> {
    fn sample(&self) -> f64 {
        self.generator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .random()
    }
}

pub struct ArgillaLogger<T, R> {
    pub transport: T,
    pub random: R,
    pub environment: BTreeMap<String, String>,
    pub credentials: ArgillaCredentials,
    pub transformation: IndexMap<String, PayloadField>,
    pub sampling_rate: f64,
    pub queue: BatchQueue<ArgillaItem>,
}

impl<T: Transport, R: RandomSource> ArgillaLogger<T, R> {
    pub async fn connect(
        settings: ArgillaSettings,
        transport: T,
        random: R,
    ) -> Result<Self, DatasetError<T::Error>> {
        let dataset_name = resolve_dataset(&transport, &settings.credentials).await?;
        let mut credentials = settings.credentials;
        credentials.dataset_name = dataset_name;
        Ok(Self {
            transport,
            random,
            environment: settings.environment,
            credentials,
            transformation: settings.transformation,
            sampling_rate: settings.sampling_rate,
            queue: BatchQueue::new(settings.batch),
        })
    }

    pub async fn log_success_event(&self, kwargs: &CallbackKwargs) {
        let rate = digit_sampling_rate(
            self.environment
                .get(SYNC_SAMPLING_RATE_VARIABLE)
                .map(String::as_str),
        );
        if self.sampled_out(rate) {
            return;
        }
        let Ok(item) = prepare_log_data(&self.transformation, kwargs).inspect_err(|error| {
            tracing::error!(%error, "Argilla Layer Error - log_success_event error");
        }) else {
            return;
        };
        self.queue_item(item, &self.delivery(BatchMode::Sync)).await;
    }

    pub async fn async_log_success_event(
        &self,
        kwargs: &CallbackKwargs,
        hooks: &[&dyn DatasetHook],
    ) {
        if self.sampled_out(self.sampling_rate) {
            return;
        }
        let mut item = match prepare_log_data(&self.transformation, kwargs) {
            Ok(item) => item,
            Err(error) => {
                tracing::error!(
                    %error,
                    "Argilla Layer Error - error logging async success event."
                );
                return;
            }
        };
        let payload = kwargs.standard_logging_object.as_ref();
        for hook in hooks {
            // a hook that declines keeps the item unchanged, so every call hands over its own clone
            match hook.async_dataset_hook(item.clone(), payload).await {
                Ok(Some(next)) => item = next,
                Ok(None) => return,
                Err(DatasetHookError::NotImplemented) => {}
                Err(error @ DatasetHookError::Failed) => {
                    tracing::error!(
                        %error,
                        "Argilla Layer Error - error logging async success event."
                    );
                    return;
                }
            }
        }
        self.queue_item(item, &self.delivery(BatchMode::Async))
            .await;
    }

    pub async fn async_log_failure_event(&self, kwargs: &CallbackKwargs) {
        let skipped = self.random.sample() > self.sampling_rate;
        if skipped {
            tracing::info!(
                rate = self.sampling_rate,
                "Skipping Argilla logging, sampled out"
            );
            return;
        }
        tracing::info!("Argilla Failure Event Logging!");
        match prepare_log_data(&self.transformation, kwargs) {
            Ok(item) => {
                self.queue_item(item, &self.delivery(BatchMode::Async))
                    .await;
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    "Argilla Layer Error - error logging async failure event."
                );
            }
        }
    }

    fn sampled_out(&self, rate: f64) -> bool {
        let skip = self.random.sample() > rate;
        if skip {
            tracing::info!(rate, "Skipping Argilla logging, sampled out");
        }
        skip
    }

    #[must_use]
    pub async fn flush_queue(&self) -> FlushReport {
        let report = self.queue.flush(&self.delivery(BatchMode::Async)).await;
        if report.cancelled {
            tracing::warn!(
                batch = report.batch,
                requeued = report.requeued,
                "the Argilla flush was cancelled; its records stay queued"
            );
        }
        report
    }

    const fn delivery(&self, mode: BatchMode) -> ArgillaDelivery<'_, T> {
        ArgillaDelivery {
            transport: &self.transport,
            credentials: &self.credentials,
            mode,
        }
    }

    async fn queue_item<D>(&self, item: ArgillaItem, delivery: &D)
    where
        D: BatchDelivery<ArgillaItem> + Sync,
    {
        let enqueued = self.queue.enqueue(item);
        if enqueued.depth >= self.queue.settings.batch_size {
            self.queue.flush(delivery).await;
        }
    }
}
