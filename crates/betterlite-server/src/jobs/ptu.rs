pub mod alert;
pub mod attribution;
pub mod snapshot;
pub mod task;

mod tests;

use std::sync::Arc;

use betterlite_db::ptu::rollup::scheduled::PTU_ROLLUP_JOB_ID;
use futures_util::future::BoxFuture;

// B8372's scheduler runs one of these per tick; the adapter never spawns, so shutdown joins the run.
pub type JobTick = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PtuRollupSchedule {
    pub id: &'static str,
    pub crontab: &'static str,
    pub timezone: &'static str,
    pub replace_existing: bool,
    // None takes the scheduler's JobDefaults, which holds APSCHEDULER_MISFIRE_GRACE_TIME, the value LiteLLM passes here.
    pub misfire_grace_seconds: Option<u64>,
}

// LiteLLM's cron block: hour 0, minute 15, UTC, replace_existing, the scheduler's misfire grace.
pub const PTU_ROLLUP_SCHEDULE: PtuRollupSchedule = PtuRollupSchedule {
    id: PTU_ROLLUP_JOB_ID,
    crontab: "15 0 * * *",
    timezone: "UTC",
    replace_existing: true,
    misfire_grace_seconds: None,
};

pub struct PtuRollupRegistration {
    pub schedule: PtuRollupSchedule,
    pub run: JobTick,
}
