use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;

use super::FOLDED_FIELDS;
use super::KEY_REQUEST_IDENTITY;
use super::TEAM_NOT_COLUMNS;
use super::error::TeamAdminError;
use super::fields::Fields;
use super::fields::as_object;
use super::fields::metadata_changes;
use super::fields::object_eq;
use super::fields::value_eq;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Columns {
    Team,
    Key,
}

type WindowPair = (Option<JsonData>, Option<JsonData>);

// Logical fields whose stored value the projection would change, metadata folded to their own names.
pub(crate) fn changed_fields(
    submitted: &Fields,
    stored: &Fields,
    columns: Columns,
) -> Result<BTreeSet<String>, TeamAdminError> {
    let excluded: &[&str] = match columns {
        Columns::Team => &TEAM_NOT_COLUMNS,
        Columns::Key => &KEY_REQUEST_IDENTITY,
    };
    let mut changed = BTreeSet::new();
    for name in submitted.keys() {
        if !excluded.contains(&name.as_str())
            && !FOLDED_FIELDS.contains(&name.as_str())
            && column_changed(columns, name, submitted, stored)?
        {
            changed.insert(name.clone());
        }
    }
    changed.extend(metadata_changes(submitted, stored));
    Ok(changed)
}

pub(crate) fn column_changed(
    columns: Columns,
    name: &str,
    submitted: &Fields,
    stored: &Fields,
) -> Result<bool, TeamAdminError> {
    match columns {
        Columns::Team => Ok(team_column_changed(name, submitted, stored)),
        Columns::Key => key_column_changed(name, submitted, stored),
    }
}

fn team_column_changed(name: &str, submitted: &Fields, stored: &Fields) -> bool {
    if name == "model_aliases" {
        let aliases = match stored.get("litellm_model_table") {
            Some(JsonData::Object(table)) => as_object(table.get("model_aliases")),
            Some(
                JsonData::Null
                | JsonData::Bool(_)
                | JsonData::Number(_)
                | JsonData::String(_)
                | JsonData::Array(_),
            )
            | None => Fields::new(),
        };
        return !object_eq(&as_object(submitted.get(name)), &aliases);
    }
    if stored.contains_key(name) {
        return !value_eq(submitted.get(name), stored.get(name));
    }
    true
}

fn key_column_changed(
    name: &str,
    submitted: &Fields,
    stored: &Fields,
) -> Result<bool, TeamAdminError> {
    if name == "budget_limits" {
        let sent = budget_windows(submitted.get(name))?;
        let held = budget_windows(stored.get(name))?;
        if let (Some(sent), Some(held)) = (&sent, &held) {
            return Ok(!same_windows(sent, held));
        }
    }
    if stored.contains_key(name) {
        return Ok(!value_eq(submitted.get(name), stored.get(name)));
    }
    Ok(true)
}

// The caller-owned pairs of a window list, or None when the value needs a plain comparison.
fn budget_windows(value: Option<&JsonData>) -> Result<Option<Vec<WindowPair>>, TeamAdminError> {
    let Some(value) = value else {
        return Ok(Some(Vec::new()));
    };
    if matches!(value, JsonData::Null) {
        return Ok(Some(Vec::new()));
    }
    let JsonData::Array(items) = value else {
        return Ok(None);
    };
    let mut windows = Vec::with_capacity(items.len());
    for item in items {
        let JsonData::Object(fields) = item else {
            return Ok(None);
        };
        let duration = pair_value(fields.get("budget_duration"))?;
        let budget = pair_value(fields.get("max_budget"))?;
        windows.push((duration, budget));
    }
    let duplicated = windows.iter().enumerate().any(|(index, window)| {
        windows
            .iter()
            .skip(index.saturating_add(1))
            .any(|other| same_pair(window, other))
    });
    if duplicated {
        return Ok(None);
    }
    Ok(Some(windows))
}

fn pair_value(value: Option<&JsonData>) -> Result<Option<JsonData>, TeamAdminError> {
    match value {
        None | Some(JsonData::Null) => Ok(None),
        Some(JsonData::Array(_) | JsonData::Object(_)) => Err(TeamAdminError::UnhashableWindow),
        Some(JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_)) => Ok(value.cloned()),
    }
}

fn same_pair(left: &WindowPair, right: &WindowPair) -> bool {
    let durations = value_eq(left.0.as_ref(), right.0.as_ref());
    let budgets = value_eq(left.1.as_ref(), right.1.as_ref());
    durations && budgets
}

fn same_windows(sent: &[WindowPair], held: &[WindowPair]) -> bool {
    sent.len() == held.len()
        && sent
            .iter()
            .all(|window| held.iter().any(|other| same_pair(window, other)))
}
