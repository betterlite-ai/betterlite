use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;

// _parse_email_list: a list keeps its trimmed nonempty strings, a string splits on commas and every other value yields none
#[must_use]
pub fn parse_email_list(raw: &JsonData) -> Vec<String> {
    match raw {
        JsonData::Array(items) => items.iter().filter_map(string_entry).collect(),
        JsonData::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_owned)
            .collect(),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Object(_) => {
            Vec::new()
        }
    }
}

fn string_entry(value: &JsonData) -> Option<String> {
    let JsonData::String(text) = value else {
        return None;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

// _normalize_alert_emails: a falsy mapping normalizes to an empty one
#[must_use]
pub fn normalize_alert_emails(
    raw: Option<&BTreeMap<String, JsonData>>,
) -> BTreeMap<String, Vec<String>> {
    let Some(config) = raw.filter(|config| !config.is_empty()) else {
        return BTreeMap::new();
    };
    config
        .iter()
        .map(|(threshold, recipients)| (threshold.clone(), parse_email_list(recipients)))
        .collect()
}

// _merge_budget_alert_email_configs: thresholds are unioned, global recipients first then per-key ones deduplicated; two empty maps merge to None
#[must_use]
pub fn merge_alert_email_configs(
    global_config: Option<&BTreeMap<String, JsonData>>,
    per_key_config: Option<&BTreeMap<String, JsonData>>,
) -> Option<BTreeMap<String, Vec<String>>> {
    let global = normalize_alert_emails(global_config);
    let per_key = normalize_alert_emails(per_key_config);
    if global.is_empty() && per_key.is_empty() {
        return None;
    }
    let thresholds: BTreeSet<&String> = global.keys().chain(per_key.keys()).collect();
    let mut merged = BTreeMap::new();
    for threshold in thresholds {
        let combined: Vec<String> = global
            .get(threshold)
            .into_iter()
            .flatten()
            .chain(per_key.get(threshold).into_iter().flatten())
            .cloned()
            .collect();
        merged.insert(threshold.clone(), dedup_recipients(&combined));
    }
    Some(merged)
}

// list(dict.fromkeys(recipients)): the first occurrence of each recipient keeps its place
#[must_use]
pub fn dedup_recipients(recipients: &[String]) -> Vec<String> {
    let mut unique: Vec<String> = Vec::new();
    for email in recipients {
        if !unique.contains(email) {
            unique.push(email.clone());
        }
    }
    unique
}

// _is_valid_alert_threshold_pct: at most three digits naming 1..=100; betterlite rejects the non-ASCII digits str.isdigit accepts
#[must_use]
pub fn is_valid_alert_threshold_pct(pct: &str) -> bool {
    if pct.is_empty() || pct.len() > 3 || !pct.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    pct.parse::<i64>()
        .is_ok_and(|value| (1_i64..=100_i64).contains(&value))
}

// _alert_recipients: a string or list yields recipients, anything else yields None
#[must_use]
pub fn alert_recipients(raw: &JsonData) -> Option<Vec<String>> {
    match raw {
        JsonData::String(_) | JsonData::Array(_) => Some(parse_email_list(raw)),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Object(_) => None,
    }
}

// _valid_alert_threshold_config: a non-mapping is discarded and invalid thresholds dropped; an unsupported recipient list stays empty
#[must_use]
pub fn valid_alert_threshold_config(
    raw: Option<&JsonData>,
) -> Option<BTreeMap<String, Vec<String>>> {
    let Some(JsonData::Object(fields)) = raw else {
        return None;
    };
    let mut config = BTreeMap::new();
    for (threshold, recipients) in fields {
        if !is_valid_alert_threshold_pct(threshold) {
            continue;
        }
        config.insert(
            threshold.clone(),
            alert_recipients(recipients).unwrap_or_default(),
        );
    }
    Some(config)
}

// min(int(key) for key in config if key.isdigit()): no 1..=100 bound applies here
#[must_use]
pub fn min_digit_threshold(config: &BTreeMap<String, Vec<String>>) -> Option<f64> {
    config
        .keys()
        .filter(|threshold| threshold.bytes().all(|byte| byte.is_ascii_digit()))
        .filter_map(|threshold| threshold.parse::<f64>().ok())
        .min_by(f64::total_cmp)
}
