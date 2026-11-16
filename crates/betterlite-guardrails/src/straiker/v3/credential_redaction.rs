use betterlite_prompts::arguments::json::{Json, Object};

use crate::straiker::v3::{CREDENTIAL_FIELDS, REDACTED_VALUE};

// Credentials ride on a tool or MCP server entry itself, so the scrub is one level deep: a function schema that defines a `token` or `headers` property lives under `parameters` and is relayed as sent.
#[must_use]
pub fn without_credentials(entries: &Json) -> Json {
    let Json::List(items) = entries else {
        return entries.clone();
    };
    Json::List(
        items
            .iter()
            .map(|entry| match entry {
                Json::Object(fields) => Json::Object(redact_entry(fields)),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    entry.clone()
                }
            })
            .collect(),
    )
}

fn redact_entry(fields: &Object) -> Object {
    fields
        .iter()
        .map(|(key, value)| {
            let redacted = CREDENTIAL_FIELDS.contains(&key.to_lowercase().as_str());
            let relayed = if redacted {
                Json::Text(REDACTED_VALUE.to_owned())
            } else {
                value.clone()
            };
            (key.clone(), relayed)
        })
        .collect()
}

mod tests;
