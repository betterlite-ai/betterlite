use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::credential::{
    CredentialBase, CredentialItem, CredentialItemFields,
};

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

pub(crate) fn masked_credential(credential: CredentialItem) -> CredentialItem {
    CredentialItem {
        fields: CredentialItemFields {
            base: CredentialBase {
                credential_name: credential.fields.base.credential_name,
                credential_info: credential.fields.base.credential_info,
            },
            credential_values: Payload(mask_fields(&credential.fields.credential_values.0, 0)),
        },
    }
}

pub(crate) fn mask_fields(
    fields: &BTreeMap<String, JsonData>,
    depth: usize,
) -> BTreeMap<String, JsonData> {
    fields
        .iter()
        .map(|(key, value)| {
            let lowered = key.to_lowercase();
            let sensitive = SENSITIVE_KEYWORDS
                .iter()
                .any(|keyword| lowered.contains(keyword));
            let masked = if sensitive {
                mask_value(value, depth)
            } else {
                value.clone()
            };
            (key.clone(), masked)
        })
        .collect()
}

fn mask_value(value: &JsonData, depth: usize) -> JsonData {
    match value {
        JsonData::Object(fields) => {
            if depth >= MAX_MASK_DEPTH {
                value.clone()
            } else {
                JsonData::Object(mask_fields(fields, depth.saturating_add(1)))
            }
        }
        JsonData::String(text) => JsonData::String(mask_text(text)),
        _ => value.clone(),
    }
}

fn mask_text(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= UNMASKED_LENGTH {
        // LiteLLM answers five asterisks for a value four characters or shorter
        return "*****".to_owned();
    }
    let head: String = characters.iter().take(VISIBLE_CHARS).collect();
    let tail: String = characters
        .iter()
        .skip(characters.len().saturating_sub(VISIBLE_CHARS))
        .collect();
    format!("{head}{}{tail}", "*".repeat(ASTERISKS))
}
