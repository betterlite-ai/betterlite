use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, SecondsFormat, Utc};
use sha2::{Digest as _, Sha256};

// Only a key-management write stamps settings_updated_at; updated_at belongs to every spend flush.
#[must_use]
pub fn with_settings_updated_at(
    data: &BTreeMap<String, JsonData>,
    now: DateTime<Utc>,
) -> BTreeMap<String, JsonData> {
    let mut fields = data.clone();
    fields.insert(
        "settings_updated_at".to_owned(),
        JsonData::String(now.to_rfc3339_opts(SecondsFormat::AutoSi, false)),
    );
    fields
}

// The sk- prefix check is case-sensitive, and every other string, Bearer or JWT text and "" passes through.
#[must_use]
pub fn hash_token_if_needed(token: &str) -> String {
    if token.starts_with("sk-") {
        hex::encode(Sha256::digest(token.as_bytes()))
    } else {
        token.to_owned()
    }
}
