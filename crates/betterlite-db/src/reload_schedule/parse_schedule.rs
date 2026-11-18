use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::JsonData;
use crate::resource_repository::config::decode_config_json;

use super::ReloadSchedule;

mod tests;

// The LiteLLM_Config row as row_to_json returns it: param_value holds the serialized schedule
#[derive(Debug, Clone, Deserialize)]
pub struct ReloadConfigRow {
    pub param_value: JsonData,
    pub last_run_at: Option<NaiveDateTime>,
    pub reload_revision: Option<i64>,
}

// _IntervalConfig: the one field LiteLLM validates, in the shape the row stores it
#[derive(Debug, Deserialize, Serialize)]
pub struct IntervalConfig {
    pub interval_hours: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseScheduleError {
    #[error("the stored reload revision {value} does not fit a u32")]
    Revision { value: i64 },
}

pub fn parse_reload_schedule(row: &ReloadConfigRow) -> Result<ReloadSchedule, ParseScheduleError> {
    let reload_revision = match row.reload_revision {
        Some(value) => {
            u32::try_from(value).map_err(|_out_of_range| ParseScheduleError::Revision { value })?
        }
        None => 0,
    };
    Ok(ReloadSchedule {
        interval_hours: interval_hours(&row.param_value),
        reload_revision,
        last_run_at: row.last_run_at.map(NaiveDateTime::and_utc),
    })
}

// param_value is serialized JSON, and a raw row read can hand it back decoded or still as a string
fn interval_hours(param_value: &JsonData) -> Option<u32> {
    let decoded;
    let value = match param_value {
        JsonData::String(text) => {
            decoded = decode_config_json(text.as_bytes()).ok()?;
            &decoded
        }
        other => other,
    };
    let text = serde_json::to_string(value).ok()?;
    let config: IntervalConfig = serde_json::from_str(&text).ok()?;
    config.interval_hours
}
