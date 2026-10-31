use chrono::{DateTime, Utc};

pub mod apply;
pub mod clamp;
pub mod identity;
pub mod offset;
pub mod staggerable;
pub mod timing;
pub mod window;

pub const MONTHLY_SPEND_REPORT_JOB_ID: &str = "monthly_spend_report_job";
pub const FALLBACK_STATS_JOB_ID: &str = "prometheus_fallback_stats_job";
pub const PTU_ROLLUP_JOB_ID: &str = "ptu_flat_cost_rollup_job";
pub const PTU_ROLLUP_LOCK_TTL_SECONDS: u32 = 900;

pub type SubmissionListener = Box<dyn Fn(&dyn JobSubmission) + Send + Sync>;

pub trait Trigger: Send + Sync {
    fn next_fire_time(
        &self,
        previous_fire_time: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>>;
}

// APScheduler tells self-anchored jobs apart by whether next_run_time was passed at all, not its value; Unset carries "attribute absent".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextRunTime {
    Unset,
    Explicit(Option<DateTime<Utc>>),
}

pub struct JobSnapshot {
    pub id: String,
    pub next_run_time: NextRunTime,
    pub period_seconds: Option<u32>,
    pub trigger: Box<dyn Trigger>,
}

pub trait JobScheduler {
    fn running(&self) -> bool;
    fn jobs(&mut self) -> Vec<JobSnapshot>;
    fn modify_job(&mut self, job_id: &str, trigger: Box<dyn Trigger>);
    fn add_submission_listener(&mut self, listener: SubmissionListener);
}

pub trait JobSubmission {
    fn job_id(&self) -> &str;
    fn scheduled_run_times(&self) -> &[DateTime<Utc>];
}
