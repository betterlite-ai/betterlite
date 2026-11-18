use std::sync::Arc;

use async_trait::async_trait;

use betterlite_logging::finops::cloudzero::delivery::transport::{BillingTransport, Clock};

use super::export::initialize_export_job;
use super::{CloudZeroLogger, EXPORT_JOB_NAME};

// The scheduler is injected; an interval job is the only thing the export needs from it.
pub trait CloudZeroScheduler {
    fn add_interval_job(
        &mut self,
        job_id: &str,
        interval_minutes: i64,
        job: Arc<dyn CloudZeroExportJob>,
    );
}

#[async_trait]
pub trait CloudZeroExportJob: Send + Sync {
    async fn run(&self);
}

#[async_trait]
impl<Sender, Time> CloudZeroExportJob for CloudZeroLogger<Sender, Time>
where
    Sender: BillingTransport + Clone + Send + Sync + 'static,
    Time: Clock + Clone + 'static,
{
    async fn run(&self) {
        if let Err(error) = initialize_export_job(self).await {
            tracing::error!(%error, "CloudZero Logger: the usage data export failed");
        }
    }
}

// init_cloudzero_background_job: the first CloudZero logger on the configured interval, or nothing.
pub fn init_cloudzero_background_job<Sender, Time>(
    scheduler: &mut dyn CloudZeroScheduler,
    loggers: &[Arc<CloudZeroLogger<Sender, Time>>],
) where
    Sender: BillingTransport + Clone + Send + Sync + 'static,
    Time: Clock + Clone + 'static,
{
    let Some(logger) = loggers.first() else {
        tracing::debug!("found no cloudzero loggers");
        return;
    };
    tracing::debug!(
        interval_minutes = logger.settings.interval_minutes,
        "CloudZero Logger: initializing the usage data export job"
    );
    scheduler.add_interval_job(
        EXPORT_JOB_NAME,
        logger.settings.interval_minutes,
        Arc::clone(logger),
    );
}
