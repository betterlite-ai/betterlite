use betterlite_core::budget_reset::compute_budget_reset_at;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::search_tools::convert::isoformat;
use chrono::{DateTime, Utc};

use crate::key_policy_fields::{
    ClockError, KeyClock, microsecond_precision, shifted_by, zulu_datetime,
};

use super::super::Row;
use super::super::error::KeyPatchError;

// duration and budget_duration feed expires and budget_reset_at; null clears them, "" leaves the stored column alone.
pub(super) fn apply_request_times(
    values: &mut Row,
    clock: KeyClock<'_>,
) -> Result<(), KeyPatchError> {
    match values.remove("duration") {
        Some(JsonData::String(text)) => {
            if text == "-1" {
                values.insert(String::from("expires"), JsonData::Null);
            } else if !text.is_empty() {
                let expires = zulu_datetime(shifted_by(clock.now, &text, clock)?);
                values.insert(String::from("expires"), JsonData::String(expires));
            }
        }
        Some(JsonData::Null) => {
            values.insert(String::from("expires"), JsonData::Null);
        }
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
        )
        | None => {}
    }
    match values.remove("budget_duration") {
        Some(JsonData::String(text)) if !text.is_empty() => {
            let reset = microsecond_precision(budget_reset(&text, clock)?);
            values.insert(
                String::from("budget_reset_at"),
                JsonData::String(zulu_datetime(reset)),
            );
            values.insert(String::from("budget_duration"), JsonData::String(text));
        }
        Some(JsonData::Null) => {
            values.insert(String::from("budget_duration"), JsonData::Null);
            values.insert(String::from("budget_reset_at"), JsonData::Null);
        }
        Some(
            JsonData::String(_)
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => {}
    }
    Ok(())
}

// A nonempty window list re-times every window and serializes; [] and null store the JSON literal null.
pub(super) fn apply_budget_windows(
    values: &mut Row,
    clock: KeyClock<'_>,
) -> Result<(), KeyPatchError> {
    let Some(windows) = values.get("budget_limits").cloned() else {
        return Ok(());
    };
    let JsonData::Array(entries) = windows else {
        values.insert(
            String::from("budget_limits"),
            JsonData::String(String::from("null")),
        );
        return Ok(());
    };
    if entries.is_empty() {
        values.insert(
            String::from("budget_limits"),
            JsonData::String(String::from("null")),
        );
        return Ok(());
    }
    let mut initialized = Vec::with_capacity(entries.len());
    for entry in entries {
        initialized.push(initialize_window(entry, clock)?);
    }
    values.insert(
        String::from("budget_limits"),
        JsonData::String(serde_json::to_string(&JsonData::Array(initialized))?),
    );
    Ok(())
}

// Every kept window keeps its fields and takes the reset time its own budget_duration computes.
fn initialize_window(entry: JsonData, clock: KeyClock<'_>) -> Result<JsonData, KeyPatchError> {
    let mut fields = match entry {
        JsonData::Object(fields) => fields,
        other @ (JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_)) => return Ok(other),
    };
    let Some(JsonData::String(spec)) = fields.get("budget_duration").cloned() else {
        return Ok(JsonData::Object(fields));
    };
    let reset = budget_reset(&spec, clock)?;
    fields.insert(
        String::from("reset_at"),
        JsonData::String(isoformat(microsecond_precision(reset))),
    );
    Ok(JsonData::Object(fields))
}

fn budget_reset(spec: &str, clock: KeyClock<'_>) -> Result<DateTime<Utc>, ClockError> {
    compute_budget_reset_at(spec, clock.reset, clock.now, clock.database)
        .map_err(|source| ClockError::Duration {
            spec: spec.to_owned(),
            source,
        })
        .map(|reset| reset.instant)
}
