use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};
use betterlite_models::capture_rate::alert::alert_message;
use betterlite_models::capture_rate::compute::CaptureRateError;
use betterlite_models::capture_rate::report::CheckResult;
use betterlite_models::capture_rate::settings::{
    SpendCaptureProvider, SpendCaptureRateCheckSettings,
};
use betterlite_provider::billing::BillingHttpGet;
use chrono::{Days, NaiveDate, Utc};
use rust_decimal::Decimal;

use self::credentials::{BillingCredentialError, BillingCredentials};
use self::lease::claims_alert_window;
use self::report::{ProviderCheck, capture_rate_report};
use self::store::{CapturedSpendError, CapturedSpendSource};

pub mod credentials;
pub mod lease;
pub mod report;
pub mod store;

mod tests;

pub trait CaptureRatePublisher: Send + Sync {
    fn publish(&self, provider: SpendCaptureProvider, rate: Option<Decimal>);
}

#[async_trait]
pub trait CaptureRateAlert: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn alert(&self, message: &str) -> Result<(), Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureRateJobError<E>
where
    E: StdError + 'static,
{
    #[error("the billing admin credential could not be read")]
    Credentials(#[source] BillingCredentialError),
    #[error(transparent)]
    Spend(#[from] CapturedSpendError),
    #[error(transparent)]
    Compute(#[from] CaptureRateError),
    #[error("the capture-rate window cannot be represented on the calendar")]
    DateRange,
    #[error("the capture-rate lookback must be at least one day")]
    Lookback,
    #[error("the capture-rate alert could not be sent")]
    Alert(#[source] E),
}

pub struct CaptureRateJob<'deps, E: StdError + Send + Sync + 'static> {
    pub spend: &'deps dyn CapturedSpendSource,
    pub http_get: &'deps dyn BillingHttpGet,
    pub credentials: &'deps dyn BillingCredentials,
    pub publisher: &'deps dyn CaptureRatePublisher,
    pub alert: &'deps dyn CaptureRateAlert<Error = E>,
}

pub async fn run_check<E>(
    job: &CaptureRateJob<'_, E>,
    settings: &SpendCaptureRateCheckSettings,
    today: Option<NaiveDate>,
) -> Result<Vec<CheckResult>, CaptureRateJobError<E>>
where
    E: StdError + Send + Sync + 'static,
{
    let results = check_every_provider(job, settings, today).await?;
    for message in results.iter().filter_map(alert_message) {
        job.alert
            .alert(&message)
            .await
            .map_err(CaptureRateJobError::Alert)?;
    }
    Ok(results)
}

// Every worker publishes its own gauge; the first replica whose finished check has an alert claims the window.
pub async fn run_scheduled_check<E, C>(
    job: &CaptureRateJob<'_, E>,
    settings: &SpendCaptureRateCheckSettings,
    pod_lock_manager: Option<&PodLockManager<C>>,
    today: Option<NaiveDate>,
) -> Result<Vec<CheckResult>, CaptureRateJobError<E>>
where
    E: StdError + Send + Sync + 'static,
    C: LockCache,
{
    let results = check_every_provider(job, settings, today).await?;
    let messages: Vec<String> = results.iter().filter_map(alert_message).collect();
    if messages.is_empty() {
        return Ok(results);
    }
    if !claims_alert_window(pod_lock_manager).await {
        tracing::info!("Spend capture-rate check: another pod alerted this window");
        return Ok(results);
    }
    for message in messages {
        job.alert
            .alert(&message)
            .await
            .map_err(CaptureRateJobError::Alert)?;
    }
    Ok(results)
}

async fn check_every_provider<E>(
    job: &CaptureRateJob<'_, E>,
    settings: &SpendCaptureRateCheckSettings,
    today: Option<NaiveDate>,
) -> Result<Vec<CheckResult>, CaptureRateJobError<E>>
where
    E: StdError + Send + Sync + 'static,
{
    let check_date = today.unwrap_or_else(|| Utc::now().date_naive());
    let end_date = check_date
        .checked_sub_days(Days::new(1))
        .ok_or(CaptureRateJobError::DateRange)?;
    let days_before = settings
        .lookback_days
        .checked_sub(1)
        .and_then(|days| u64::try_from(days).ok())
        .ok_or(CaptureRateJobError::Lookback)?;
    let start_date = end_date
        .checked_sub_days(Days::new(days_before))
        .ok_or(CaptureRateJobError::DateRange)?;
    let mut results = Vec::with_capacity(settings.providers.len());
    for provider in &settings.providers {
        let check = ProviderCheck {
            provider: *provider,
            start_date,
            end_date,
            threshold: settings.threshold,
            openai_project_ids: &settings.openai_project_ids,
        };
        results.push(capture_rate_report(job, &check).await?);
    }
    for result in &results {
        job.publisher
            .publish(result.provider(), result.published_rate());
        tracing::info!(
            provider = ?result.provider(),
            rate = ?result.published_rate(),
            "spend capture-rate check finished"
        );
    }
    Ok(results)
}
