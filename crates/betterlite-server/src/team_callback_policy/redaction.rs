use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_secrets::callback_vars::is_sensitive_callback_key;
use betterlite_secrets::secret_codec::CALLBACK_VAR_ENCRYPTED_PREFIX;
use indexmap::IndexMap;

use crate::team_callback_policy::CALLBACK_VARS_REDACTED;

pub const LOGGING_KEY: &str = "logging";
pub const CALLBACK_SETTINGS_KEY: &str = "callback_settings";
const CALLBACK_VARS_KEY: &str = "callback_vars";

// _redact_callback_secrets: the audit copy keeps which variables changed but none of their values
#[must_use]
pub fn redact_callback_secrets(metadata: &Json) -> Json {
    let Json::Object(fields) = metadata else {
        return metadata.clone();
    };
    let mut redacted = fields.clone();
    if let Some(Json::List(entries)) = redacted.get_mut(LOGGING_KEY) {
        for entry in entries.iter_mut() {
            redact_slot(entry);
        }
    }
    if let Some(settings) = redacted.get_mut(CALLBACK_SETTINGS_KEY) {
        redact_slot(settings);
    }
    Json::Object(redacted)
}

fn redact_slot(slot: &mut Json) {
    let Json::Object(fields) = slot else {
        return;
    };
    let Some(Json::Object(vars)) = fields.get_mut(CALLBACK_VARS_KEY) else {
        return;
    };
    for value in vars.values_mut() {
        *value = Json::Text(CALLBACK_VARS_REDACTED.to_owned());
    }
}

// _mask_sensitive_callback_vars: credentials and values that failed to decrypt are hidden, plain configuration stays readable
pub fn mask_sensitive_callback_vars(settings: &mut IndexMap<String, Json>) {
    let Some(Json::Object(vars)) = settings.get_mut(CALLBACK_VARS_KEY) else {
        return;
    };
    let no_extra_keys = BTreeSet::new();
    for (key, value) in vars.iter_mut() {
        let Json::Text(text) = value else {
            continue;
        };
        if is_sensitive_callback_key(key, &no_extra_keys)
            || text.starts_with(CALLBACK_VAR_ENCRYPTED_PREFIX)
        {
            *value = Json::Text(CALLBACK_VARS_REDACTED.to_owned());
        }
    }
}
