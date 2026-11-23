use betterlite_core::budget_reset::compute_budget_reset_at;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::search_tools::convert::isoformat;

use crate::key_policy_fields::{
    ClockError, KeyClock, microsecond_precision, shifted_by, zulu_datetime,
};

use super::super::Row;
use super::super::error::KeyPatchError;

// duration is request-only: it becomes expires, or clears it for null and "-1"; "" leaves the column alone.
pub(super) fn apply_duration(values: &mut Row, clock: KeyClock<'_>) -> Result<(), KeyPatchError> {
    let Some(duration) = values.remove("duration") else {
        return Ok(());
    };
    match duration {
        JsonData::Null => {
            values.insert(String::from("expires"), JsonData::Null);
        }
        JsonData::String(text) if text == "-1" => {
            values.insert(String::from("expires"), JsonData::Null);
        }
        JsonData::String(text) if !text.is_empty() => {
            let instant = shifted_by(clock.now, &text, clock)?;
            values.insert(
                String::from("expires"),
                JsonData::String(zulu_datetime(instant)),
            );
        }
        JsonData::String(_)
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => {}
    }
    Ok(())
}

// budget_duration null clears the stored duration and its reset; "" leaves both as stored.
pub(super) fn apply_budget_duration(
    values: &mut Row,
    clock: KeyClock<'_>,
) -> Result<(), KeyPatchError> {
    let Some(budget_duration) = values.remove("budget_duration") else {
        return Ok(());
    };
    match budget_duration {
        JsonData::Null => {
            values.insert(String::from("budget_duration"), JsonData::Null);
            values.insert(String::from("budget_reset_at"), JsonData::Null);
        }
        JsonData::String(text) if text.is_empty() => {}
        JsonData::String(text) => {
            let reset = compute_budget_reset_at(&text, clock.reset, clock.now, clock.database)
                .map_err(|source| ClockError::Duration {
                    spec: text.clone(),
                    source,
                })?;
            values.insert(
                String::from("budget_reset_at"),
                JsonData::String(zulu_datetime(microsecond_precision(reset.instant))),
            );
            values.insert(String::from("budget_duration"), JsonData::String(text));
        }
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_) => {}
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
    let reset = compute_budget_reset_at(&spec, clock.reset, clock.now, clock.database)
        .map_err(|source| ClockError::Duration { spec, source })?;
    fields.insert(
        String::from("reset_at"),
        JsonData::String(isoformat(microsecond_precision(reset.instant))),
    );
    Ok(JsonData::Object(fields))
}
