use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

pub type DbCipher = StoredCipher<OsNonces, TracingDiagnostics>;
pub const V2_GCM_PREFIX: &str = "v2:gcm:";
pub const MAX_MIGRATION_JSON_BYTES: usize = 0x0010_0000;

pub mod report;
use report::{LocationReport, MigrationReport, ValueClass};

pub struct CoveredTableSpec {
    pub location: &'static str,
    pub table: &'static str,
    pub json_columns: &'static [&'static str],
    pub scalar_columns: &'static [&'static str],
}

pub const COVERED_TABLE_SPECS: &[CoveredTableSpec] = &[
    CoveredTableSpec {
        location: "model_table",
        table: "litellm_proxymodeltable",
        json_columns: &["litellm_params"],
        scalar_columns: &[],
    },
    CoveredTableSpec {
        location: "credentials",
        table: "litellm_credentialstable",
        json_columns: &["credential_values"],
        scalar_columns: &[],
    },
    CoveredTableSpec {
        location: "mcp_server",
        table: "litellm_mcpservertable",
        json_columns: &["credentials", "env_vars", "static_headers", "env"],
        scalar_columns: &[],
    },
    CoveredTableSpec {
        location: "mcp_user_credentials",
        table: "litellm_mcpusercredentials",
        json_columns: &[],
        scalar_columns: &["credential_b64"],
    },
    CoveredTableSpec {
        location: "mcp_user_env_vars",
        table: "litellm_mcpuserenvvars",
        json_columns: &[],
        scalar_columns: &["values_b64"],
    },
];

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("counter overflow")]
    CounterOverflow,
    #[error("stored migration JSON exceeds {MAX_MIGRATION_JSON_BYTES} bytes")]
    InputTooLarge,
    #[error("invalid stored migration JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("database scan read failed at {location}")]
    Read {
        location: String,
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("encryption failed")]
    Encrypt(#[source] SecretError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MigrationJson {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(SecretText),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretText(String);

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretText(REDACTED)")
    }
}

impl SecretText {
    #[must_use]
    pub const fn new(value: String) -> Self {
        if value.is_empty() {
            return Self(value);
        }
        Self(value)
    }
}

pub fn decode_migration_json(bytes: &[u8]) -> Result<MigrationJson, ScanError> {
    if bytes.len() > MAX_MIGRATION_JSON_BYTES {
        return Err(ScanError::InputTooLarge);
    }
    Ok(serde_json::from_slice(bytes)?)
}

#[must_use]
pub fn classify_value(value: &MigrationJson, cipher: &DbCipher, key: &str) -> ValueClass {
    let MigrationJson::String(text) = value else {
        return ValueClass::NotAString;
    };
    if text.0.starts_with(V2_GCM_PREFIX) {
        return ValueClass::Migrated;
    }
    if text.0.is_empty() {
        return ValueClass::Plaintext;
    }
    let stored = cipher.decrypt_value(
        StoredValue::<()>::Text(SecretString::from(text.0.as_str())),
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

pub fn reencrypt_value(
    value: &MigrationJson,
    cipher: &DbCipher,
    key: &str,
) -> Result<MigrationJson, ScanError> {
    let MigrationJson::String(text) = value else {
        return Ok(value.clone());
    };
    if text.0.is_empty() || text.0.starts_with(V2_GCM_PREFIX) {
        return Ok(value.clone());
    }
    let Some(StoredValue::Text(plain)) = cipher.decrypt_value(
        StoredValue::<()>::Text(SecretString::from(text.0.as_str())),
        key,
        DecryptPolicy {
            log: FailureLog::Debug,
            value: FailureValue::Nothing,
        },
    ) else {
        return Ok(value.clone());
    };
    match cipher.encrypt_value(StoredValue::Text(plain), None) {
        Ok(StoredValue::Text(sealed)) => Ok(MigrationJson::String(SecretText::new(
            sealed.expose_secret().to_owned(),
        ))),
        Ok(StoredValue::Other(())) => Ok(value.clone()),
        Err(source) => Err(ScanError::Encrypt(source)),
    }
}

pub fn iter_encrypted_strings(value: &MigrationJson) -> impl Iterator<Item = &SecretText> {
    let mut stack = vec![value];
    let mut leaves = Vec::new();
    while let Some(current) = stack.pop() {
        match current {
            MigrationJson::String(text) => leaves.push(text),
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
pub fn classify_callback_value(value: &MigrationJson, cipher: &DbCipher) -> ValueClass {
    let MigrationJson::String(text) = value else {
        return ValueClass::NotAString;
    };
    let inner = text
        .0
        .strip_prefix("litellm_enc::")
        .unwrap_or(text.0.as_str());
    classify_value(
        &MigrationJson::String(SecretText::new(inner.to_owned())),
        cipher,
        "callback",
    )
}

#[must_use]
pub fn classify_secret_map(
    value: &MigrationJson,
    cipher: &DbCipher,
    _key: &str,
) -> Option<ValueClass> {
    let text = match value {
        MigrationJson::String(text) => text.0.clone(),
        MigrationJson::Object(values) => match serde_json::to_string(values) {
            Ok(text) => text,
            Err(_) => return Some(ValueClass::Undecryptable),
        },
        MigrationJson::Null
        | MigrationJson::Bool(_)
        | MigrationJson::Number(_)
        | MigrationJson::Array(_) => return Some(ValueClass::Undecryptable),
    };
    let unquoted = serde_json::from_str::<String>(&text).unwrap_or(text);
    if unquoted.starts_with(V2_GCM_PREFIX) {
        return Some(ValueClass::Migrated);
    }
    let Ok(Some(decoded)) = cipher.decode_secret_map(unquoted.as_bytes()) else {
        return Some(ValueClass::Undecryptable);
    };
    if decoded.is_empty() {
        return None;
    }
    Some(if unquoted.starts_with(V2_GCM_PREFIX) {
        ValueClass::Migrated
    } else {
        ValueClass::Legacy
    })
}

pub mod storage;

mod tests;
