use chrono::{DateTime, Utc};
use serde::Serialize;

pub mod evictor;
pub mod next_run_at;
pub mod parse_schedule;
pub mod pod_reload_decision;
pub mod status_formatting;
pub mod store;

#[path = "reload_schedule/tests/bounded_writes.rs"]
mod bounded_writes;

#[path = "reload_schedule/tests/clear_interval.rs"]
mod clear_interval;

#[path = "reload_schedule/tests/eviction_failures.rs"]
mod eviction_failures;

#[path = "reload_schedule/tests/manual_reload_atomic.rs"]
mod manual_reload_atomic;

#[path = "reload_schedule/tests/read_from_db.rs"]
mod read_from_db;

#[path = "reload_schedule/tests/record_run.rs"]
mod record_run;

#[path = "reload_schedule/tests/support.rs"]
mod support;

#[path = "reload_schedule/tests/timezone_handling.rs"]
mod timezone_handling;

#[path = "reload_schedule/tests/validation_errors.rs"]
mod validation_errors;

#[path = "reload_schedule/tests/write_interval.rs"]
mod write_interval;

// MODEL_COST_MAP_RELOAD_PARAM_NAME: the one LiteLLM_Config row this module reads and writes
pub const RELOAD_CONFIG_PARAM_NAME: &str = "model_cost_map_reload_config";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadSchedule {
    pub interval_hours: Option<u32>,
    pub reload_revision: u32,
    pub last_run_at: Option<DateTime<Utc>>,
}

// ReloadScheduleStatus: the fields the admin status route merges with the cost map provenance
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReloadScheduleStatus {
    pub scheduled: bool,
    pub interval_hours: Option<u32>,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
}
