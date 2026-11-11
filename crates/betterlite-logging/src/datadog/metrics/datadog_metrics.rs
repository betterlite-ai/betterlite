use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::batch::logger::BatchQueue;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::datadog_logging::transport::Transport;

use super::clock::Clock;
use super::error::MetricsError;
use super::flush::{flush_queue, periodic_flush};
use super::measurements::{
    MetricSource, RequestTiming, SUCCESS_STATUS_CODE, failure_status_code, metrics_from_log,
};
use super::series::DatadogMetricSeries;
use super::settings::{MetricsInit, MetricsSettings};

mod tests;

pub struct MetricsDeps {
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    // the server owns the flush timer's task; without a tracker only explicit flushes run
    pub tasks: Option<TaskTracker>,
}

pub struct DatadogMetricsLogger {
    pub settings: MetricsSettings,
    pub queue: BatchQueue<DatadogMetricSeries>,
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    pub stopped: CancellationToken,
}

impl DatadogMetricsLogger {
    pub fn new(
        environment: &dyn EnvironmentLookup,
        init: &MetricsInit,
        deps: MetricsDeps,
    ) -> Result<Arc<Self>, MetricsError> {
        let settings = MetricsSettings::resolve(environment, init)?;
        let logger = Arc::new(Self {
            queue: BatchQueue::new(settings.batch),
            settings,
            transport: deps.transport,
            clock: deps.clock,
            stopped: CancellationToken::new(),
        });
        match deps.tasks {
            Some(tasks) => {
                let periodic = Arc::clone(&logger);
                let tracker = tasks.clone();
                let _task = tasks.spawn(async move { periodic_flush(periodic, tracker).await });
            }
            None => {
                tracing::debug!(
                    "Datadog Metrics: no task tracker, the periodic flush did not start"
                );
            }
        }
        Ok(logger)
    }

    pub async fn record(&self, source: &MetricSource<'_>) {
        let now = self.clock.now_seconds();
        let series =
            match metrics_from_log(source, self.settings.batch.flush_interval.as_secs(), now) {
                Ok(series) => series,
                Err(error) => {
                    tracing::error!(%error, "Datadog Metrics Layer Error");
                    return;
                }
            };
        let mut depth = 0;
        for entry in series {
            depth = self.queue.enqueue(entry).depth;
        }
        if depth >= self.settings.batch.batch_size {
            let _report = flush_queue(self).await;
        }
    }
}

#[async_trait]
impl CallbackLogger for DatadogMetricsLogger {
    fn callback_name(&self) -> &'static str {
        "datadog_metrics"
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let source = MetricSource {
            environment: &self.settings.environment,
            payload: event.payload.as_ref(),
            timing: RequestTiming::from_payload(event.payload.as_ref()),
            status_code: SUCCESS_STATUS_CODE.to_owned(),
        };
        self.record(&source).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let source = MetricSource {
            environment: &self.settings.environment,
            payload: event.payload.as_ref(),
            timing: RequestTiming::from_payload(event.payload.as_ref()),
            status_code: failure_status_code(event.payload.as_ref()),
        };
        self.record(&source).await;
        Ok(())
    }
}
