use std::error::Error as StdError;
use std::ops::{Add as _, Sub as _};

use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_logging::alert_settings::cache_keys::SlackAlertingCacheKeys;
use betterlite_logging::latency_alerts::api_base::ApiBaseResolver;
use betterlite_models::api_base_lookup::ApiBaseError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use self::daily_report::{
    DailyReportRouter, daily_report_message, failed_requests_section, latency_section,
    reset_entries,
};
use self::daily_report_scheduler::ReportJitter;
use self::deployment_metrics::{DailyReportCache, metric_keys, metric_number};
use crate::alerting::model_alerts::ModelAlertSender;
use crate::alerting::model_alerts::model_deprecation::{AlertPodLock, AlertSleeper};
use crate::model_deprecation::clock::UtcClock;

pub mod daily_report;
pub mod daily_report_scheduler;
pub mod deployment_metrics;
pub mod deployment_success_logging;

mod tests;

// SLACK_DAILY_REPORT_LOCK_ID: the cronjob id the report's pod lock is held under.
pub const SLACK_DAILY_REPORT_LOCK_ID: &str = "slack_daily_report";

const JITTER_SECONDS: i64 = 3;

#[derive(Debug, thiserror::Error)]
#[error("the clock instant is not a finite number of seconds")]
pub struct StampError;

#[derive(Debug, thiserror::Error)]
pub enum DailyReportError<E>
where
    E: StdError + 'static,
{
    #[error("the daily report could not be sent: {0}")]
    Alert(#[source] E),
    #[error("the api base of a deployment could not be resolved: {0}")]
    ApiBase(#[from] ApiBaseError),
    #[error(transparent)]
    Stamp(#[from] StampError),
}

// The clock read a report makes: the seconds the interval check compares, and the same instant as the cache stamp.
#[derive(Debug)]
struct ReportInstant {
    seconds: f64,
    stamp: Json,
}

pub struct DeploymentReports<Cache, Sender, ApiBase, Clock, Sleeper, Jitter> {
    pub alert_types: Vec<AlertType>,
    pub alerting_args: SlackAlertingArgs,
    pub cache: Cache,
    pub sender: Sender,
    pub api_base: ApiBase,
    pub clock: Clock,
    pub sleeper: Sleeper,
    pub jitter: Jitter,
}

impl<Cache, Sender, ApiBase, Clock, Sleeper, Jitter>
    DeploymentReports<Cache, Sender, ApiBase, Clock, Sleeper, Jitter>
where
    Cache: DailyReportCache,
    Sender: ModelAlertSender,
    ApiBase: ApiBaseResolver,
    Clock: UtcClock + Send + Sync,
    Sleeper: AlertSleeper,
    Jitter: ReportJitter,
{
    // send_daily_reports: one batch read, the top-five message, the reset, then the alert.
    pub async fn send_daily_reports<Router>(
        &self,
        router: &Router,
    ) -> Result<bool, DailyReportError<Sender::Error>>
    where
        Router: DailyReportRouter,
    {
        let keys = metric_keys(&router.model_ids());
        let Some(raw) = self.cache.batch_get(&keys.combined).await else {
            return Ok(false);
        };
        let values: Vec<Option<f64>> = raw
            .iter()
            .map(|value| value.as_ref().and_then(metric_number))
            .collect();
        if !values
            .iter()
            .any(|value| value.is_some_and(|number| number > 0.0_f64))
        {
            return Ok(false);
        }
        let (failed_values, latency_values) = values.split_at(keys.failed.len());
        let Some(failed_rows) =
            failed_requests_section(&self.api_base, router, &keys.failed, failed_values)?
        else {
            return Ok(false);
        };
        let latency_rows = latency_section(&self.api_base, router, &keys.latency, latency_values)?;
        let now = self.clock_instant()?;
        let next_run = now
            .seconds
            .add(f64::from(self.alerting_args.daily_report_frequency));
        let message = daily_report_message(&failed_rows, &latency_rows, now.seconds, next_run);
        reset_entries(&self.cache, &keys).await;
        self.send_alert(&message)
            .await
            .map_err(DailyReportError::Alert)?;
        Ok(true)
    }

    // _run_scheduler_helper: stamps the window on the first pass, sends inside the interval afterwards.
    pub async fn run_scheduler_helper<Router, Lock>(
        &self,
        router: &Router,
        pod_lock: Option<&Lock>,
    ) -> Result<bool, DailyReportError<Sender::Error>>
    where
        Router: DailyReportRouter,
        Lock: AlertPodLock,
    {
        let key = SlackAlertingCacheKeys::DailyMetricsReportSent.to_string();
        let stored = self.cache.get(&key).await;
        let now = self.clock_instant()?;
        let Some(stamp) = stored else {
            self.cache.set(&key, &now.stamp).await;
            return Ok(false);
        };
        let Some(last_sent) = metric_number(&stamp) else {
            return Ok(false);
        };
        let interval = f64::from(self.alerting_args.daily_report_frequency);
        if now.seconds.sub(last_sent) < interval {
            return Ok(false);
        }
        if !self.claim_report_lock(pod_lock).await {
            return Ok(false);
        }
        self.send_daily_reports(router).await?;
        self.cache.set(&key, &now.stamp).await;
        Ok(true)
    }

    // _run_scheduled_daily_report: the report loop, polling a jittered check interval until shutdown.
    pub async fn run_scheduled_daily_report<Router, Lock>(
        &self,
        router: Option<&Router>,
        pod_lock: Option<&Lock>,
    ) where
        Router: DailyReportRouter,
        Lock: AlertPodLock,
    {
        let Some(active_router) = router else {
            return;
        };
        if !self.alert_types.contains(&AlertType::DailyReports) {
            return;
        }
        loop {
            if let Err(error) = self.run_scheduler_helper(active_router, pod_lock).await {
                tracing::error!("Error in the daily report scheduler: {error}");
            }
            let check_interval = i64::from(self.alerting_args.report_check_interval);
            let low = check_interval.saturating_sub(JITTER_SECONDS);
            let high = check_interval.saturating_add(JITTER_SECONDS);
            let seconds = self.jitter.between(low, high);
            if !self
                .sleeper
                .sleep(u64::try_from(seconds).unwrap_or_default())
                .await
            {
                return;
            }
        }
    }

    async fn send_alert(&self, message: &str) -> Result<(), Sender::Error> {
        let metadata = IndexMap::new();
        let kwargs = IndexMap::new();
        let alert = Alert {
            message,
            level: AlertLevel::Low,
            alert_type: AlertType::DailyReports,
            alerting_metadata: &metadata,
            user_info: None,
            request_model: None,
            api_base: None,
            kwargs: &kwargs,
        };
        self.sender.send_alert(alert).await
    }

    // is False from acquire_lock refuses the send; a missing lock and a None answer both go ahead, as in LiteLLM.
    async fn claim_report_lock<Lock>(&self, pod_lock: Option<&Lock>) -> bool
    where
        Lock: AlertPodLock,
    {
        let Some(lock) = pod_lock else {
            return true;
        };
        let ttl = i64::from(self.alerting_args.daily_report_frequency);
        lock.acquire_lock(SLACK_DAILY_REPORT_LOCK_ID, Some(ttl), false)
            .await
            != Some(false)
    }

    fn clock_instant(&self) -> Result<ReportInstant, StampError> {
        let seconds = self.clock.now().timestamp();
        let value = num_traits::cast::<i64, f64>(seconds).ok_or(StampError)?;
        let number = Number::from_f64(value).ok_or(StampError)?;
        Ok(ReportInstant {
            seconds: value,
            stamp: Json::Number(number),
        })
    }
}
