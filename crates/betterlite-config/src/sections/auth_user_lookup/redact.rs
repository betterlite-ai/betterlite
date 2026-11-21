use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_redact::structured::{REDACTED, SensitiveDataMasker};
use indexmap::IndexMap;

const MAX_REDACT_DEPTH: usize = 10;
// proxy_server.py's _EXTRA_SECRET_GENERAL_SETTINGS_FIELDS
const EXTRA_SECRET_FIELDS: [&str; 4] = [
    "database_url",
    "database_extra_connection_params",
    "pass_through_endpoints",
    "alert_to_webhook_url",
];

// proxy_server.py's _redact_secret_values_in_obj: a sensitive key's value becomes REDACTED, deeper fails closed
#[must_use]
pub(super) fn redacted_params(values: &IndexMap<String, JsonData>) -> JsonData {
    let masker = SensitiveDataMasker::default();
    let mut redacted = BTreeMap::new();
    for (key, value) in values {
        let kept =
            if EXTRA_SECRET_FIELDS.contains(&key.as_str()) || masker.is_sensitive_key(key, &[]) {
                JsonData::String(REDACTED.to_owned())
            } else {
                redacted_value(&masker, value, 1)
            };
        redacted.insert(key.clone(), kept);
    }
    JsonData::Object(redacted)
}

fn redacted_value(masker: &SensitiveDataMasker, value: &JsonData, depth: usize) -> JsonData {
    if depth >= MAX_REDACT_DEPTH {
        return JsonData::String(REDACTED.to_owned());
    }
    match value {
        JsonData::Object(fields) => JsonData::Object(
            fields
                .iter()
                .map(|(name, child)| {
                    if EXTRA_SECRET_FIELDS.contains(&name.as_str())
                        || masker.is_sensitive_key(name, &[])
                    {
                        return (name.clone(), JsonData::String(REDACTED.to_owned()));
                    }
                    (
                        name.clone(),
                        redacted_value(masker, child, depth.saturating_add(1)),
                    )
                })
                .collect(),
        ),
        JsonData::Array(items) => JsonData::Array(
            items
                .iter()
                .map(|item| redacted_value(masker, item, depth.saturating_add(1)))
                .collect(),
        ),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) => {
            value.clone()
        }
    }
}
