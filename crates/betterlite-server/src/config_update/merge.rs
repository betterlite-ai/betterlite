use betterlite_config::sections::settings::JsonValue;

use super::SettingsMap;

const ALERTING: &str = "alerting";
const ALERT_TO_WEBHOOK_URL: &str = "alert_to_webhook_url";
const SUCCESS_CALLBACK: &str = "success_callback";
const SLACK: &str = "slack";

// requested_general_settings: ConfigGeneralSettings.model_dump(exclude_none=True, exclude_unset=True)
#[must_use]
pub fn without_nulls(values: &SettingsMap) -> SettingsMap {
    values
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

// every sent key lands first, then the webhook side effect appends slack to the sent alerting list whatever order the request used
#[must_use]
pub fn general_settings(existing: &SettingsMap, sent: &SettingsMap) -> SettingsMap {
    let mut merged = existing.clone();
    for (key, value) in sent {
        merged.insert(key.clone(), value.clone());
    }
    if sent.contains_key(ALERT_TO_WEBHOOK_URL) {
        enable_slack_alerting(&mut merged);
    }
    merged
}

fn enable_slack_alerting(merged: &mut SettingsMap) {
    let slack = JsonValue::String(SLACK.to_owned());
    if !merged.contains_key(ALERTING) {
        merged.insert(ALERTING.to_owned(), JsonValue::Array(vec![slack]));
        return;
    }
    if let Some(JsonValue::Array(items)) = merged.get_mut(ALERTING) {
        if !items.iter().any(|item| item.as_str() == Some(SLACK)) {
            items.push(slack);
        }
    }
}

// existing.update(_encrypt_env_variables_for_db(environment_variables)): only the sent keys are rewritten
#[must_use]
pub fn environment_variables(existing: &SettingsMap, sealed: &SettingsMap) -> SettingsMap {
    let mut merged = existing.clone();
    for (key, value) in sealed {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

// updated_litellm_settings normalizes success_callback, then merged holds request wins and the callbacks are additive
#[must_use]
pub fn litellm_settings(existing: &SettingsMap, raw: &SettingsMap) -> SettingsMap {
    let mut updated = raw.clone();
    if let Some(JsonValue::Array(incoming)) = raw.get(SUCCESS_CALLBACK) {
        updated.insert(
            SUCCESS_CALLBACK.to_owned(),
            JsonValue::Array(normalize_callback_names(incoming)),
        );
    }
    let mut merged = existing.clone();
    for (key, value) in &updated {
        merged.insert(key.clone(), value.clone());
    }
    if let Some(JsonValue::Array(incoming)) = updated.get(SUCCESS_CALLBACK) {
        let union = match existing.get(SUCCESS_CALLBACK) {
            Some(JsonValue::Array(stored)) => {
                let mut names = normalize_callback_names(stored);
                names.extend(incoming.iter().cloned());
                names
            }
            None
            | Some(
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::String(_)
                | JsonValue::Object(_),
            ) => incoming.clone(),
        };
        merged.insert(SUCCESS_CALLBACK.to_owned(), JsonValue::Array(dedupe(union)));
    }
    merged
}

// normalize_callback_names: a string callback is lowercased, anything else is kept as sent
#[must_use]
pub fn normalize_callback_names(callbacks: &[JsonValue]) -> Vec<JsonValue> {
    callbacks
        .iter()
        .map(|callback| match callback {
            JsonValue::String(name) => JsonValue::String(name.to_lowercase()),
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_) => callback.clone(),
        })
        .collect()
}

// list(set(...)): the order is not kept, so this keeps the first occurrence of each entry
fn dedupe(callbacks: Vec<JsonValue>) -> Vec<JsonValue> {
    let mut unique: Vec<JsonValue> = Vec::with_capacity(callbacks.len());
    for callback in callbacks {
        if !unique.contains(&callback) {
            unique.push(callback);
        }
    }
    unique
}

#[must_use]
pub fn router_settings(existing: &SettingsMap, updates: &SettingsMap) -> SettingsMap {
    let mut merged = existing.clone();
    for (key, value) in updates {
        merged.insert(key.clone(), value.clone());
    }
    merged
}
