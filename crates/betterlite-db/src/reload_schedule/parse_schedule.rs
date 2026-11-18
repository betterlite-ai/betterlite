use chrono::NaiveDateTime;
use serde::Deserialize;

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

#[must_use]
pub fn parse_reload_schedule(row: &ReloadConfigRow) -> ReloadSchedule {
    let revision = row.reload_revision.unwrap_or_default();
    ReloadSchedule {
        interval_hours: interval_hours(&row.param_value),
        reload_revision: u32::try_from(revision).unwrap_or_default(),
        last_run_at: row.last_run_at.map(NaiveDateTime::and_utc),
    }
}

// param_value is serialized JSON, and a raw row read can hand it back decoded or still as a string
fn interval_hours(param_value: &JsonData) -> Option<u32> {
    let decoded;
    let config = match param_value {
        JsonData::String(text) => {
            decoded = decode_config_json(text.as_bytes()).ok()?;
            &decoded
        }
        other => other,
    };
    let JsonData::Object(fields) = config else {
        return None;
    };
    let Some(JsonData::Number(number)) = fields.get("interval_hours") else {
        return None;
    };
    number.as_u64().and_then(|value| u32::try_from(value).ok())
}
