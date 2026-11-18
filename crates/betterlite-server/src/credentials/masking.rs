use std::collections::BTreeMap;

use betterlite_config::credentials::{CredentialItem, CredentialValues};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;

use super::wire::CredentialView;

const SENSITIVE_KEYWORDS: [&str; 8] = [
    "authorization",
    "token",
    "key",
    "secret",
    "vertex_credentials",
    "credentials",
    "password",
    "passwd",
];
const MAX_MASK_DEPTH: usize = 20;
const UNMASKED_LENGTH: usize = 4;
const VISIBLE_CHARS: usize = 2;
const ASTERISKS: usize = 4;
// past the depth limit the container itself is replaced, so a deep value can never answer plaintext
const REDACTED: &str = "*****";

pub(super) fn masked_credential(credential: &CredentialItem) -> CredentialView {
    CredentialView {
        name: credential.credential_name.clone(),
        values: masked_values(&credential.credential_values),
        info: credential.credential_info.clone(),
    }
}

pub(crate) fn masked_values(values: &CredentialValues) -> Payload {
    let plain = Payload(
        values
            .0
            .iter()
            .map(|(key, value)| (key.clone(), value.expose_json()))
            .collect(),
    );
    mask_object(&plain, MAX_MASK_DEPTH)
}

// containers_left is the number of nested objects and arrays the walk may still enter
fn mask_object(fields: &Payload, containers_left: usize) -> Payload {
    let mut masked = BTreeMap::new();
    for (key, value) in &fields.0 {
        let exposed = mask_json(value, containers_left, is_sensitive(key));
        masked.insert(key.clone(), exposed);
    }
    Payload(masked)
}

fn is_sensitive(key: &str) -> bool {
    let lowered = key.to_lowercase();
    SENSITIVE_KEYWORDS
        .iter()
        .any(|keyword| lowered.contains(keyword))
}

// mask_strings covers an array under a sensitive key, where every string is part of the secret
fn mask_json(value: &JsonData, containers_left: usize, mask_strings: bool) -> JsonData {
    match value {
        JsonData::Object(fields) => containers_left.checked_sub(1).map_or_else(
            || JsonData::String(REDACTED.to_owned()),
            |left| {
                let nested = Payload(fields.clone());
                JsonData::Object(mask_object(&nested, left).0)
            },
        ),
        JsonData::Array(items) => containers_left.checked_sub(1).map_or_else(
            || JsonData::String(REDACTED.to_owned()),
            |left| {
                JsonData::Array(
                    items
                        .iter()
                        .map(|item| mask_json(item, left, mask_strings))
                        .collect(),
                )
            },
        ),
        JsonData::String(text) if mask_strings => JsonData::String(mask_text(text)),
        JsonData::String(text) => JsonData::String(text.clone()),
        leaf @ (JsonData::Null | JsonData::Bool(_) | JsonData::Number(_)) => leaf.clone(),
    }
}

fn mask_text(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= UNMASKED_LENGTH {
        // LiteLLM answers five asterisks for a value four characters or shorter
        return REDACTED.to_owned();
    }
    let head: String = characters.iter().take(VISIBLE_CHARS).collect();
    let tail: String = characters
        .iter()
        .skip(characters.len().saturating_sub(VISIBLE_CHARS))
        .collect();
    format!("{head}{}{tail}", "*".repeat(ASTERISKS))
}
