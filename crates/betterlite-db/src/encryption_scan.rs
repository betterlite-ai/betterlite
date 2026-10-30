use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type DbCipher = StoredCipher<OsNonces, TracingDiagnostics>;
pub const V2_GCM_PREFIX: &str = "v2:gcm:";
pub const MAX_MIGRATION_JSON_BYTES: usize = 0x0010_0000;

pub const COVERED_TABLE_SPECS: &[(&str, &str, &[&str], &[&str])] = &[
    ("model_table", "litellm_params", &["litellm_params"], &[]),
    (
        "credentials",
        "credential_values",
        &["credential_values"],
        &[],
    ),
    (
        "mcp_server",
        "credentials",
        &["credentials", "env_vars", "static_headers", "env"],
        &[],
    ),
    (
        "mcp_user_credentials",
        "credential_b64",
        &[],
        &["credential_b64"],
    ),
    ("mcp_user_env_vars", "values_b64", &[], &["values_b64"]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ValueClass {
    Migrated,
    Legacy,
    Plaintext,
    Undecryptable,
    NotAString,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationReport {
    pub location: String,
    pub scanned: u64,
    pub migrated: u64,
    pub already_v2: u64,
    pub plaintext: u64,
    pub undecryptable: u64,
    pub legacy: u64,
}

impl LocationReport {
    #[must_use]
    pub const fn new(location: String) -> Self {
        let scanned = 0;
        Self {
            location,
            scanned,
            migrated: scanned,
            already_v2: scanned,
            plaintext: scanned,
            undecryptable: scanned,
            legacy: scanned,
        }
    }

    pub fn count(&mut self, class: ValueClass) -> Result<(), ScanError> {
        self.scanned = self
            .scanned
            .checked_add(1)
            .ok_or(ScanError::CounterOverflow)?;
        let counter = match class {
            ValueClass::Migrated => &mut self.already_v2,
            ValueClass::Legacy => &mut self.legacy,
            ValueClass::Undecryptable => &mut self.undecryptable,
            ValueClass::Plaintext | ValueClass::NotAString => &mut self.plaintext,
        };
        *counter = counter.checked_add(1).ok_or(ScanError::CounterOverflow)?;
        Ok(())
    }

    #[must_use]
    pub fn as_dict(&self) -> BTreeMap<String, u64> {
        let mut fields = BTreeMap::new();
        fields.insert(String::from("scanned"), self.scanned);
        fields.insert(String::from("migrated"), self.migrated);
        fields.insert(String::from("already_v2"), self.already_v2);
        fields.insert(String::from("plaintext"), self.plaintext);
        fields.insert(String::from("undecryptable"), self.undecryptable);
        fields.insert(String::from("legacy"), self.legacy);
        fields
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReport {
    pub locations: Vec<LocationReport>,
}

impl MigrationReport {
    pub fn residual_legacy(&self) -> Result<u64, ScanError> {
        match sum_reports(&self.locations, |report| report.legacy) {
            Ok(total) => Ok(total),
            Err(error) => Err(error),
        }
    }

    pub fn total_undecryptable(&self) -> Result<u64, ScanError> {
        match sum_reports(&self.locations, |report| report.undecryptable) {
            Ok(total) => Ok(total),
            Err(error) => Err(error),
        }
    }
}

fn sum_reports(
    reports: &[LocationReport],
    field: fn(&LocationReport) -> u64,
) -> Result<u64, ScanError> {
    let mut total = 0_u64;
    for report in reports {
        total = total
            .checked_add(field(report))
            .ok_or(ScanError::CounterOverflow)?;
    }
    Ok(total)
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("counter overflow")]
    CounterOverflow,
    #[error("stored migration JSON exceeds {MAX_MIGRATION_JSON_BYTES} bytes")]
    InputTooLarge,
    #[error("invalid stored migration JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MigrationJson {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

pub fn decode_migration_json(bytes: &[u8]) -> Result<MigrationJson, ScanError> {
    if bytes.len() > MAX_MIGRATION_JSON_BYTES {
        return Err(ScanError::InputTooLarge);
    }
    Ok(serde_json::from_slice(bytes)?)
}

#[must_use]
pub fn is_migrated(value: &str) -> bool {
    let prefix_matches = value.starts_with(V2_GCM_PREFIX);
    if prefix_matches {
        return true;
    }
    false
}

#[must_use]
pub fn classify_value(value: &MigrationJson, cipher: &DbCipher, key: &str) -> ValueClass {
    let MigrationJson::String(text) = value else {
        return ValueClass::NotAString;
    };
    if is_migrated(text) {
        return ValueClass::Migrated;
    }
    if text.is_empty() {
        return ValueClass::Plaintext;
    }
    let stored = cipher.decrypt_value(
        StoredValue::<()>::Text(SecretString::from(text.as_str())),
        key,
        DecryptPolicy {
            log: FailureLog::Debug,
            value: FailureValue::Nothing,
        },
    );
    if stored.is_some() {
        ValueClass::Legacy
    } else {
        ValueClass::Plaintext
    }
}

#[must_use]
pub fn reencrypt_value(value: &MigrationJson, cipher: &DbCipher, key: &str) -> MigrationJson {
    let MigrationJson::String(text) = value else {
        return value.clone();
    };
    if text.is_empty() || is_migrated(text) {
        return value.clone();
    }
    let Some(StoredValue::Text(plain)) = cipher.decrypt_value(
        StoredValue::<()>::Text(SecretString::from(text.as_str())),
        key,
        DecryptPolicy {
            log: FailureLog::Debug,
            value: FailureValue::Nothing,
        },
    ) else {
        return value.clone();
    };
    match cipher.encrypt_value(StoredValue::Text(plain), None) {
        Ok(StoredValue::Text(sealed)) => MigrationJson::String(sealed.expose_secret().to_owned()),
        Ok(StoredValue::Other(())) | Err(_) => value.clone(),
    }
}

pub fn iter_encrypted_strings(value: &MigrationJson) -> impl Iterator<Item = &str> {
    let mut stack = vec![value];
    let mut leaves = Vec::new();
    while let Some(current) = stack.pop() {
        match current {
            MigrationJson::String(text) => leaves.push(text.as_str()),
            MigrationJson::Array(values) => stack.extend(values.iter()),
            MigrationJson::Object(values) => stack.extend(values.values()),
            MigrationJson::Null | MigrationJson::Bool(_) | MigrationJson::Number(_) => {}
        }
    }
    leaves.into_iter()
}

pub fn _iter_encrypted_strings(value: &MigrationJson) -> impl Iterator<Item = &str> {
    let mut stack = vec![value];
    let mut leaves = Vec::new();
    while let Some(current) = stack.pop() {
        match current {
            MigrationJson::String(text) => leaves.push(text.as_str()),
            MigrationJson::Array(values) => stack.extend(values.iter()),
            MigrationJson::Object(values) => stack.extend(values.values()),
            MigrationJson::Null | MigrationJson::Bool(_) | MigrationJson::Number(_) => {}
        }
    }
    leaves.into_iter()
}

#[must_use]
pub fn _iter_callback_var_dicts(value: &MigrationJson) -> Vec<&BTreeMap<String, MigrationJson>> {
    let mut found = Vec::new();
    if let MigrationJson::Object(root) = value {
        if let Some(MigrationJson::Array(entries)) = root.get("logging") {
            found.extend(entries.iter().filter_map(callback_vars_from_entry));
        }
        if let Some(settings) = root.get("callback_settings")
            && let Some(callback_vars) = callback_vars_from_entry(settings)
        {
            found.push(callback_vars);
        }
    }
    found
}

fn callback_vars_from_entry(value: &MigrationJson) -> Option<&BTreeMap<String, MigrationJson>> {
    let MigrationJson::Object(entry) = value else {
        return None;
    };
    let MigrationJson::Object(callback_vars) = entry.get("callback_vars")? else {
        return None;
    };
    Some(callback_vars)
}

#[must_use]
pub fn _classify_callback_value(value: &MigrationJson, cipher: &DbCipher) -> ValueClass {
    let MigrationJson::String(text) = value else {
        return ValueClass::NotAString;
    };
    let inner = text.strip_prefix("litellm_enc::").unwrap_or(text);
    classify_value(&MigrationJson::String(inner.to_owned()), cipher, "callback")
}

#[must_use]
pub fn _classify_secret_map(value: &MigrationJson, cipher: &DbCipher, key: &str) -> ValueClass {
    if matches!(value, MigrationJson::Object(values) if values.is_empty()) {
        return ValueClass::Plaintext;
    }
    let MigrationJson::String(text) = value else {
        return ValueClass::Undecryptable;
    };
    let Ok(decoded) = cipher.decode_secret_map(text.as_bytes()) else {
        return ValueClass::Undecryptable;
    };
    if decoded.as_ref().is_some_and(BTreeMap::is_empty) {
        return ValueClass::Plaintext;
    }
    classify_value(&MigrationJson::String(text.to_owned()), cipher, key)
}

mod tests;
