use chrono::{DateTime, TimeDelta, Utc};
use secrecy::ExposeSecret as _;

use betterlite_db::finops::cloudzero::get_usage_data;
use betterlite_logging::finops::cloudzero::delivery::transport::{BillingTransport, Clock};
use betterlite_logging::finops::cloudzero::delivery::{CloudZeroStreamer, Credentials};
use betterlite_logging::finops::cloudzero::records::cbf::CbfTransformer;

use super::error::CloudZeroExportError;
use super::usage_rows::usage_records;
use super::{CloudZeroLogger, EXPORT_JOB_NAME, ExportOperation};

pub async fn export_usage_data<Sender, Time>(
    logger: &CloudZeroLogger<Sender, Time>,
    limit: Option<i64>,
    operation: Option<ExportOperation>,
    start_time_utc: Option<DateTime<Utc>>,
    end_time_utc: Option<DateTime<Utc>>,
) -> Result<(), CloudZeroExportError<Sender::Error>>
where
    Sender: BillingTransport + Clone + Sync,
    Time: Clock + Clone,
{
    let credentials = match (&logger.settings.api_key, &logger.settings.connection_id) {
        (Some(api_key), Some(connection_id))
            if !api_key.expose_secret().is_empty() && !connection_id.expose_secret().is_empty() =>
        {
            Credentials {
                api_key: api_key.clone(),
                connection_id: connection_id.clone(),
            }
        }
        _ => return Err(CloudZeroExportError::MissingConfiguration),
    };
    let usage = get_usage_data(&logger.pipeline.store, limit, start_time_utc, end_time_utc).await?;
    if usage.is_empty() {
        tracing::debug!("CloudZero Logger: no usage data found to export");
        return Ok(());
    }
    let records = usage_records(&usage)?;
    let cbf_data = CbfTransformer::default().transform(&records);
    if cbf_data.is_empty() {
        tracing::warn!("CloudZero Logger: no valid data after transformation");
        return Ok(());
    }
    let streamer = CloudZeroStreamer::new(
        credentials,
        Some(logger.settings.timezone.as_str()),
        &logger.pipeline.timezone_database,
        logger.pipeline.clock.clone(),
        logger.pipeline.transport.clone(),
    );
    tracing::debug!(
        records = cbf_data.len(),
        "CloudZero Logger: transmitting records"
    );
    streamer
        .send_batched(&cbf_data, Some(operation.unwrap_or_default().as_str()))
        .await?;
    Ok(())
}

pub async fn hourly_usage_data_export<Sender, Time>(
    logger: &CloudZeroLogger<Sender, Time>,
) -> Result<(), CloudZeroExportError<Sender::Error>>
where
    Sender: BillingTransport + Clone + Sync,
    Time: Clock + Clone,
{
    let current_time_utc = logger.pipeline.clock.now();
    let lookback_minutes = logger
        .settings
        .interval_minutes
        .checked_mul(2)
        .ok_or(CloudZeroExportError::WindowOutOfRange)?;
    let lookback =
        TimeDelta::try_minutes(lookback_minutes).ok_or(CloudZeroExportError::WindowOutOfRange)?;
    let start_time_utc = current_time_utc
        .checked_sub_signed(lookback)
        .ok_or(CloudZeroExportError::WindowOutOfRange)?;
    export_usage_data(
        logger,
        Some(logger.settings.max_fetched_records),
        Some(ExportOperation::ReplaceHourly),
        Some(start_time_utc),
        Some(current_time_utc),
    )
    .await
}

pub async fn initialize_export_job<Sender, Time>(
    logger: &CloudZeroLogger<Sender, Time>,
) -> Result<(), CloudZeroExportError<Sender::Error>>
where
    Sender: BillingTransport + Clone + Sync,
    Time: Clock + Clone,
{
    match logger
        .lease
        .as_deref()
        .filter(|lease| lease.redis_available())
    {
        None => hourly_usage_data_export(logger).await,
        Some(lease) => {
            if lease.acquire_lock(EXPORT_JOB_NAME).await != Some(true) {
                return Ok(());
            }
            let outcome = hourly_usage_data_export(logger).await;
            lease.release_lock(EXPORT_JOB_NAME).await;
            outcome
        }
    }
}
