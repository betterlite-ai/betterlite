use std::collections::BTreeSet;
use std::fmt::Display;
use std::time::SystemTime;

use betterlite_db::daily_activity_contracts::identifiers::DailyActivityTable;
use betterlite_db::daily_activity_contracts::{DailyActivityScope, DailyActivityScopeInput};
use betterlite_server::daily_activity::recovery::scope::{
    ScopeIds, ScopeRequest, daily_activity_scope,
};
use betterlite_server::daily_activity::recovery::window::spend_logs_window;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SpendLogsWindow {
        dates: Vec<Option<String>>,
    },
    DailyActivityScope {
        table: DailyActivityTable,
        entity_id_field: String,
        entity_id: Option<ScopeIdInput>,
        exclude_entity_ids: Vec<String>,
        api_key: Option<ScopeIdInput>,
        start_date: String,
        end_date: String,
        model: Option<String>,
        timezone_offset_minutes: Option<i64>,
        include_current_utc_day: bool,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum ScopeIdInput {
    One(String),
    Many(Vec<String>),
}

impl From<&ScopeIdInput> for ScopeIds {
    fn from(input: &ScopeIdInput) -> Self {
        match input {
            ScopeIdInput::One(value) => Self::One(value.clone()),
            ScopeIdInput::Many(values) => Self::Many(values.clone()),
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::SpendLogsWindow { dates } => window(dates),
            Input::DailyActivityScope {
                table,
                entity_id_field,
                entity_id,
                exclude_entity_ids,
                api_key,
                start_date,
                end_date,
                model,
                timezone_offset_minutes,
                include_current_utc_day,
            } => {
                let request = ScopeRequest {
                    table: *table,
                    entity_id_field: entity_id_field.as_str(),
                    entity_id: entity_id.as_ref().map(ScopeIds::from),
                    exclude_entity_ids: exclude_entity_ids.clone(),
                    api_key: api_key.as_ref().map(ScopeIds::from),
                    start_date: start_date.as_str(),
                    end_date: end_date.as_str(),
                    model: model.as_deref(),
                    timezone_offset_minutes: *timezone_offset_minutes,
                    include_current_utc_day: *include_current_utc_day,
                };
                let scope = daily_activity_scope(request).map_err(failure)?;
                scope_value(&scope)
            }
        }
    }
}

fn window(dates: &[Option<String>]) -> HelperResult {
    let parsed: BTreeSet<String> = dates.iter().flatten().cloned().collect();
    match spend_logs_window(&parsed) {
        Some((start, end)) => Ok(json!({"window": [text(start), text(end)]})),
        None => Ok(json!({"window": null})),
    }
}

fn text(time: SystemTime) -> String {
    DateTime::<Utc>::from(time)
        .format("%Y-%m-%dT%H:%M:%S%.6f")
        .to_string()
}

fn scope_value(scope: &DailyActivityScope) -> HelperResult {
    let input: &DailyActivityScopeInput = scope.as_ref();
    Ok(json!({
        "scope": {
            "table": input.table.metadata().table_name,
            "entity_id_field": input.entity_id_field,
            "entity_ids": input.entity_ids,
            "exclude_entity_ids": input.exclude_entity_ids,
            "api_keys": input.api_keys,
            "start_date": input.start_date,
            "end_date": input.end_date,
            "model": input.model,
            "timezone_offset_minutes": input.timezone_offset_minutes,
            "include_current_utc_day": input.include_current_utc_day,
        }
    }))
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
