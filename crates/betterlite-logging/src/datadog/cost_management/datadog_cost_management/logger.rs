use std::future::Future;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, PoisonError};

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use jiff::tz::TimeZone;
use rust_decimal::Decimal;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::aggregate;
use super::error::CostManagementError;
use super::settings::{CostManagementInit, CostManagementSettings};
use super::transport::Transport;
use super::upload;
use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::batch::logger::{BatchQueue, FlushReport};
use crate::callback_logger::event::LoggerEvent;
use crate::callback_logger::{CallbackLogger, error::LoggerError};
use crate::fields::payload::StandardLoggingPayload;

pub const CALLBACK_NAME: &str = "datadog_cost_management";

pub struct CostManagementDeps<T: Transport> {
    pub transport: Arc<T>,
    pub zone: TimeZone,
    pub tasks: Option<TaskTracker>,
    pub shutdown: CancellationToken,
}

pub struct DatadogCostManagementLogger<T: Transport> {
    pub settings: CostManagementSettings,
    pub queue: BatchQueue<Arc<StandardLoggingPayload>>,
    pub transport: Arc<T>,
    pub zone: TimeZone,
    pub stopped: CancellationToken,
}

impl<T: Transport + 'static> DatadogCostManagementLogger<T> {
    pub fn new(
        init: &CostManagementInit,
        environment: &dyn EnvironmentLookup,
        deps: CostManagementDeps<T>,
    ) -> Result<Arc<Self>, CostManagementError> {
        let settings = CostManagementSettings::resolve(init, environment)?;
        let logger = Arc::new(Self {
            queue: BatchQueue::new(settings.batch),
            settings,
            transport: deps.transport,
            zone: deps.zone,
            stopped: deps.shutdown.clone(),
        });
        if let Some(tasks) = deps.tasks {
            let periodic = Arc::clone(&logger);
            let tracker = tasks.clone();
            tasks.spawn(async move { periodic_flush(&periodic, &tracker).await });
        }
        Ok(logger)
    }

    // LiteLLM enqueues only a positive response cost, so a zero or negative one never reaches a batch
    pub async fn log_success(&self, payload: Arc<StandardLoggingPayload>) -> bool {
        if payload.response_cost <= Decimal::ZERO {
            return false;
        }
        let enqueued = self.queue.enqueue(payload);
        if enqueued.depth >= self.settings.batch.batch_size {
            self.flush_queue().await;
        }
        true
    }

    #[must_use]
    pub fn queue_length(&self) -> usize {
        self.queue
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub async fn flush_queue(&self) -> FlushReport {
        let report = self.queue.flush(&CostDelivery { logger: self }).await;
        tracing::debug!(
            batch = report.batch,
            delivered = report.delivered,
            requeued = report.requeued,
            "Datadog Cost Management: flushed the cost queue"
        );
        report
    }
}

async fn periodic_flush<T>(logger: &Arc<DatadogCostManagementLogger<T>>, tasks: &TaskTracker)
where
    T: Transport + 'static,
{
    loop {
        let shutdown = timeout(
            logger.settings.batch.flush_interval,
            logger.stopped.cancelled(),
        )
        .await;
        if shutdown.is_ok() || tasks.is_closed() {
            return;
        }
        logger.flush_queue().await;
    }
}

struct CostDelivery<'logger, T: Transport> {
    logger: &'logger DatadogCostManagementLogger<T>,
}

impl<T: Transport> BatchDelivery<Arc<StandardLoggingPayload>> for CostDelivery<'_, T> {
    fn deliver(
        &self,
        batch: &[Arc<StandardLoggingPayload>],
        _settled: &AtomicUsize,
    ) -> impl Future<
        Output = Result<
            Vec<Arc<StandardLoggingPayload>>,
            BatchCancelled<Arc<StandardLoggingPayload>>,
        >,
    > + Send {
        let logger = self.logger;
        async move {
            let entries = aggregate::aggregate(batch, &logger.settings, &logger.zone);
            if entries.is_empty() {
                tracing::debug!(
                    records = batch.len(),
                    "Datadog Cost Management: the batch held no aggregable cost; dropping it"
                );
                return Ok(Vec::new());
            }
            match upload::upload(&logger.settings, logger.transport.as_ref(), &entries).await {
                Ok(()) => Ok(Vec::new()),
                Err(error) => {
                    tracing::error!(
                        %error,
                        records = batch.len(),
                        "Datadog Cost Management: the cost upload failed; the batch stays queued"
                    );
                    Ok(batch.to_vec())
                }
            }
        }
    }
}

#[async_trait]
impl<T: Transport + 'static> CallbackLogger for DatadogCostManagementLogger<T> {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log_success(Arc::clone(&event.payload)).await;
        Ok(())
    }
}
