use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::SlackAlertingArgs;
use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};

use super::spend_reports::{DEFAULT_TIME_RANGE, parse_days};

mod tests;

pub const WEEKLY_SPEND_REPORT_JOB_ID: &str = "weekly_spend_report_job";
pub const MONTHLY_SPEND_REPORT_JOB_ID: &str = "monthly_spend_report_job";
pub const USER_SPEND_ALERTS_JOB_ID: &str = "user_spend_alerts_job";

const SECONDS_IN_A_DAY: u64 = 86_400;
const WEEKLY_LOCK_SLACK_SECONDS: u64 = 3_600;
const MONTHLY_LOCK_TTL_SECONDS: u64 = 3_600;
const USER_LOCK_SLACK_SECONDS: u64 = 60;
const MIN_USER_LOCK_SECONDS: u64 = 60;
const FIRST_RUN_BASE_SECONDS: u64 = 10;
const WEEKLY_JITTER_SECONDS: u32 = 300;
const USER_JITTER_SECONDS: u32 = 60;

#[derive(Debug, thiserror::Error)]
#[error("spend_report_frequency must be a positive number of days, e.g., '1d', '7d'")]
pub struct InvalidSpendReportFrequency;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpendJobSchedule {
    EveryDays(u32),
    EverySeconds(u32),
    MonthlyOnFirstDay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendJobSettings {
    pub id: &'static str,
    pub schedule: SpendJobSchedule,
    pub lock_ttl_seconds: u64,
    pub first_run_delay_seconds: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendJobPlan {
    pub weekly: SpendJobSettings,
    pub monthly: SpendJobSettings,
    pub user_spend: SpendJobSettings,
}

// randint(0, maximum) of the source: the composition root supplies the random draw, the plan fixes its range
pub trait FirstRunJitter: Send + Sync {
    fn offset_seconds(&self, maximum: u32) -> u32;
}

// every spend job acquires with allow_reentrant false, so the port has no flag to pass
#[async_trait]
pub trait SpendPodLock: Send + Sync {
    async fn acquire_lock(&self, job_id: &str, ttl_seconds: u64) -> Option<bool>;
}

#[async_trait]
impl<Cache: LockCache> SpendPodLock for PodLockManager<Cache> {
    async fn acquire_lock(&self, job_id: &str, ttl_seconds: u64) -> Option<bool> {
        let ttl = i64::try_from(ttl_seconds).unwrap_or(i64::MAX);
        Self::acquire_lock(self, job_id, Some(ttl), false).await
    }
}

// the three SlackAlerting methods the jobs call, so the runner holds one port instead of both producers
#[async_trait]
pub trait SpendJobTarget: Send + Sync {
    async fn weekly_spend_report(&self, time_range: &str);

    async fn monthly_spend_report(&self);

    async fn user_spend_alerts(&self, has_database: bool);
}

pub struct SpendJobRunner<L, T> {
    pub lock: L,
    pub target: T,
    pub has_database: bool,
}

impl<L, T> SpendJobRunner<L, T>
where
    L: SpendPodLock,
    T: SpendJobTarget,
{
    pub async fn run_weekly(&self, job: &SpendJobSettings, time_range: &str) {
        if self.lock.acquire_lock(job.id, job.lock_ttl_seconds).await == Some(false) {
            return;
        }
        self.target.weekly_spend_report(time_range).await;
    }

    pub async fn run_monthly(&self, job: &SpendJobSettings) {
        if self.lock.acquire_lock(job.id, job.lock_ttl_seconds).await == Some(false) {
            return;
        }
        self.target.monthly_spend_report().await;
    }

    pub async fn run_user_spend(&self, job: &SpendJobSettings) {
        if self.lock.acquire_lock(job.id, job.lock_ttl_seconds).await == Some(false) {
            return;
        }
        self.target.user_spend_alerts(self.has_database).await;
    }
}

pub fn plan_spend_jobs(
    spend_report_frequency: Option<&str>,
    args: &SlackAlertingArgs,
    jitter: &dyn FirstRunJitter,
) -> Result<SpendJobPlan, InvalidSpendReportFrequency> {
    let days = frequency_days(spend_report_frequency)?;
    let user_interval = args.user_spend_check_interval;
    Ok(SpendJobPlan {
        weekly: SpendJobSettings {
            id: WEEKLY_SPEND_REPORT_JOB_ID,
            schedule: SpendJobSchedule::EveryDays(days),
            lock_ttl_seconds: u64::from(days)
                .saturating_mul(SECONDS_IN_A_DAY)
                .saturating_sub(WEEKLY_LOCK_SLACK_SECONDS),
            first_run_delay_seconds: Some(
                FIRST_RUN_BASE_SECONDS
                    .saturating_add(u64::from(jitter.offset_seconds(WEEKLY_JITTER_SECONDS))),
            ),
        },
        monthly: SpendJobSettings {
            id: MONTHLY_SPEND_REPORT_JOB_ID,
            schedule: SpendJobSchedule::MonthlyOnFirstDay,
            lock_ttl_seconds: MONTHLY_LOCK_TTL_SECONDS,
            first_run_delay_seconds: None,
        },
        user_spend: SpendJobSettings {
            id: USER_SPEND_ALERTS_JOB_ID,
            schedule: SpendJobSchedule::EverySeconds(user_interval),
            lock_ttl_seconds: u64::from(user_interval)
                .saturating_sub(USER_LOCK_SLACK_SECONDS)
                .max(MIN_USER_LOCK_SECONDS),
            first_run_delay_seconds: Some(
                FIRST_RUN_BASE_SECONDS
                    .saturating_add(u64::from(jitter.offset_seconds(USER_JITTER_SECONDS))),
            ),
        },
    })
}

// general_settings.get("spend_report_frequency", "7d") or "7d", with the days > 0 check of the source
fn frequency_days(frequency: Option<&str>) -> Result<u32, InvalidSpendReportFrequency> {
    let frequency = frequency
        .filter(|text| !text.is_empty())
        .unwrap_or(DEFAULT_TIME_RANGE);
    let days = parse_days(frequency).map_err(|_error| InvalidSpendReportFrequency)?;
    if days == 0 {
        return Err(InvalidSpendReportFrequency);
    }
    Ok(days)
}
