use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub type DbCipher = StoredCipher<OsNonces, TracingDiagnostics>;
pub const V2_GCM_PREFIX: &str = "v2:gcm:";
pub const MAX_MIGRATION_JSON_BYTES: usize = 1_048_576;

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
    pub fn new(location: impl Into<String>) -> Self {
        let report = Self {
            location: location.into(),
            scanned: 0,
            migrated: 0,
            already_v2: 0,
            plaintext: 0,
            undecryptable: 0,
            legacy: 0,
        };
        report
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

    pub fn as_dict(&self) -> BTreeMap<String, u64> {
        let fields = BTreeMap::from([
            (String::from("scanned"), self.scanned),
            (String::from("migrated"), self.migrated),
            (String::from("already_v2"), self.already_v2),
            (String::from("plaintext"), self.plaintext),
            (String::from("undecryptable"), self.undecryptable),
            (String::from("legacy"), self.legacy),
        ]);
        fields
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReport {
    pub locations: Vec<LocationReport>,
}

impl MigrationReport {
    pub fn residual_legacy(&self) -> Result<u64, ScanError> {
        let total = sum_reports(&self.locations, |report| report.legacy);
        total
    }

    pub fn total_undecryptable(&self) -> Result<u64, ScanError> {
        let total = sum_reports(&self.locations, |report| report.undecryptable);
        total
    }
}

fn sum_reports(
    reports: &[LocationReport],
    field: fn(&LocationReport) -> u64,
) -> Result<u64, ScanError> {
    let total = reports.iter().try_fold(0_u64, |total, report| {
        total
            .checked_add(field(report))
            .ok_or(ScanError::CounterOverflow)
    });
    total
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
    Array(Vec<MigrationJson>),
    Object(BTreeMap<String, MigrationJson>),
}

pub fn decode_migration_json(bytes: &[u8]) -> Result<MigrationJson, ScanError> {
    if bytes.len() > MAX_MIGRATION_JSON_BYTES {
        return Err(ScanError::InputTooLarge);
    }
    Ok(serde_json::from_slice(bytes)?)
}

pub fn is_migrated(value: &str) -> bool {
    let migrated = value.starts_with(V2_GCM_PREFIX);
    migrated
}

pub fn classify_value(value: &Value, cipher: &DbCipher, key: &str) -> ValueClass {
    let Some(text) = value.as_str() else {
        return ValueClass::NotAString;
    };
    if is_migrated(text) {
        return ValueClass::Migrated;
    }
    if text.is_empty() {
        return ValueClass::Plaintext;
    }
    let stored = cipher.decrypt_value(stored_text(text), key, decrypt_policy());
    if stored.is_some() {
        ValueClass::Legacy
    } else {
        ValueClass::Plaintext
    }
}

pub fn reencrypt_value(value: &Value, cipher: &DbCipher, key: &str) -> Value {
    let Some(text) = value.as_str() else {
        return value.clone();
    };
    if text.is_empty() || is_migrated(text) {
        return value.clone();
    }
    let Some(StoredValue::Text(plain)) =
        cipher.decrypt_value(stored_text(text), key, decrypt_policy())
    else {
        return value.clone();
    };
    match cipher.encrypt_value(StoredValue::Text(plain), None) {
        Ok(StoredValue::Text(sealed)) => Value::String(sealed.expose_secret().to_owned()),
        Ok(StoredValue::Other(())) | Err(_) => value.clone(),
    }
}

fn stored_text(value: &str) -> StoredValue<()> {
    let stored = StoredValue::Text(SecretString::from(value));
    stored
}

fn decrypt_policy() -> DecryptPolicy {
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    policy
}

pub fn iter_encrypted_strings(value: &Value) -> impl Iterator<Item = &str> {
    let mut stack = vec![value];
    let mut leaves = Vec::new();
    while let Some(current) = stack.pop() {
        match current {
            Value::String(text) => leaves.push(text.as_str()),
            Value::Array(values) => stack.extend(values.iter()),
            Value::Object(values) => stack.extend(values.values()),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    leaves.into_iter()
}

pub fn _iter_encrypted_strings(value: &Value) -> impl Iterator<Item = &str> {
    let leaves = iter_encrypted_strings(value);
    leaves
}

pub fn _iter_callback_var_dicts(value: &Value) -> Vec<&serde_json::Map<String, Value>> {
    let mut found = Vec::new();
    if let Some(entries) = value.get("logging").and_then(Value::as_array) {
        for entry in entries {
            if let Some(callback_vars) = entry.get("callback_vars").and_then(Value::as_object) {
                found.push(callback_vars);
            }
        }
    }
    if let Some(callback_vars) = value
        .get("callback_settings")
        .and_then(|settings| settings.get("callback_vars"))
        .and_then(Value::as_object)
    {
        found.push(callback_vars);
    }
    found
}

pub fn _classify_callback_value(value: &Value, cipher: &DbCipher) -> ValueClass {
    let Some(text) = value.as_str() else {
        return ValueClass::NotAString;
    };
    let inner = text.strip_prefix("litellm_enc::").unwrap_or(text);
    classify_value(&Value::String(inner.to_owned()), cipher, "callback")
}

pub fn _classify_secret_map(value: &Value, cipher: &DbCipher, key: &str) -> ValueClass {
    if value == &Value::Object(serde_json::Map::new()) {
        return ValueClass::Plaintext;
    }
    let Some(text) = value.as_str() else {
        return ValueClass::Undecryptable;
    };
    let Ok(decoded) = cipher.decode_secret_map(text.as_bytes()) else {
        return ValueClass::Undecryptable;
    };
    if decoded.as_ref().is_some_and(BTreeMap::is_empty) {
        return ValueClass::Plaintext;
    }
    classify_value(&Value::String(text.to_owned()), cipher, key)
}

mod tests;
