use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio_postgres::GenericClient;

pub type DbCipher = StoredCipher<OsNonces, TracingDiagnostics>;
pub const V2_GCM_PREFIX: &str = "v2:gcm:";
pub const MAX_MIGRATION_JSON_BYTES: usize = 0x0010_0000;

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
    pub fn count(&mut self, class: Option<ValueClass>) -> Result<(), ScanError> {
        let Some(class) = class else { return Ok(()) };
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
        let mut total = 0_u64;
        for report in &self.locations {
            total = total
                .checked_add(report.legacy)
                .ok_or(ScanError::CounterOverflow)?;
        }
        Ok(total)
    }

    pub fn total_undecryptable(&self) -> Result<u64, ScanError> {
        let mut total = 0_u64;
        for report in &self.locations {
            total = total
                .checked_add(report.undecryptable)
                .ok_or(ScanError::CounterOverflow)?;
        }
        Ok(total)
    }

    pub fn as_dict(&self) -> Result<BTreeMap<String, serde_json::Value>, ScanError> {
        let residual = self.residual_legacy()?;
        let undecryptable = self.total_undecryptable()?;
        let locations = self
            .locations
            .iter()
            .map(|report| (report.location.clone(), serde_json::json!(report.as_dict())))
            .collect::<BTreeMap<_, _>>();
        Ok(BTreeMap::from([
            (String::from("residual_legacy"), serde_json::json!(residual)),
            (
                String::from("total_undecryptable"),
                serde_json::json!(undecryptable),
            ),
            (String::from("locations"), serde_json::json!(locations)),
        ]))
    }
}

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
    Encrypt(#[source] betterlite_secrets::error::SecretError),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MigrationJson {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(SecretString),
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
#[must_use]
pub fn classify_value(value: &MigrationJson, cipher: &DbCipher, key: &str) -> ValueClass {
    let MigrationJson::String(text) = value else {
        return ValueClass::NotAString;
    };
    if text.expose_secret().starts_with(V2_GCM_PREFIX) {
        return ValueClass::Migrated;
    }
    if text.is_empty() {
        return ValueClass::Plaintext;
    }
    let stored = cipher.decrypt_value(
        StoredValue::<()>::Text(text.clone()),
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
pub fn reencrypt_value(
    value: &MigrationJson,
    cipher: &DbCipher,
    key: &str,
) -> Result<MigrationJson, ScanError> {
    let MigrationJson::String(text) = value else {
        return Ok(value.clone());
    };
    if text.expose_secret().is_empty() || text.expose_secret().starts_with(V2_GCM_PREFIX) {
        return Ok(value.clone());
    }
    let Some(StoredValue::Text(plain)) = cipher.decrypt_value(
        StoredValue::<()>::Text(text.clone()),
        key,
        DecryptPolicy {
            log: FailureLog::Debug,
            value: FailureValue::Nothing,
        },
    ) else {
        return Ok(value.clone());
    };
    match cipher.encrypt_value(StoredValue::Text(plain), None) {
        Ok(StoredValue::Text(sealed)) => Ok(MigrationJson::String(sealed)),
        Ok(StoredValue::Other(())) => Ok(value.clone()),
        Err(source) => Err(ScanError::Encrypt(source)),
    }
}

pub fn iter_encrypted_strings(value: &MigrationJson) -> impl Iterator<Item = &SecretString> {
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
pub fn _classify_callback_value(value: &MigrationJson, cipher: &DbCipher) -> ValueClass {
    let MigrationJson::String(text) = value else {
        return ValueClass::NotAString;
    };
    let inner = text.strip_prefix("litellm_enc::").unwrap_or(text);
    classify_value(
        &MigrationJson::String(SecretString::from(inner)),
        cipher,
        "callback",
    )
}

#[must_use]
pub fn _classify_secret_map(
    value: &MigrationJson,
    cipher: &DbCipher,
    _key: &str,
) -> Option<ValueClass> {
    let text = match value {
        MigrationJson::String(text) => text.expose_secret().to_owned(),
        MigrationJson::Object(values) => match serde_json::to_string(values) {
            Ok(text) => text,
            Err(_) => return Some(ValueClass::Undecryptable),
        },
        _ => return Some(ValueClass::Undecryptable),
    };
    let unquoted = serde_json::from_str::<String>(&text).unwrap_or(text);
    if unquoted.starts_with(V2_GCM_PREFIX) {
        return Some(ValueClass::Migrated);
    }
    let decoded = match cipher.decode_secret_map(unquoted.as_bytes()) {
        Ok(Some(decoded)) => decoded,
        Ok(None) | Err(_) => return Some(ValueClass::Undecryptable),
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

fn row_value(row: &tokio_postgres::Row, column: &str) -> Option<MigrationJson> {
    if let Ok(value) = row.try_get::<_, serde_json::Value>(column) {
        return serde_json::from_value(value).ok();
    }
    row.try_get::<_, String>(column).ok().and_then(|value| {
        serde_json::from_str(&value)
            .ok()
            .or(Some(MigrationJson::String(SecretString::from(value))))
    })
}

async fn _scan_one_table<C: GenericClient + Sync>(
    client: &C,
    spec: &CoveredTableSpec,
    cipher: &DbCipher,
) -> Result<LocationReport, ScanError> {
    let columns = spec
        .json_columns
        .iter()
        .chain(spec.scalar_columns.iter())
        .map(|column| format!("\"{column}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {columns} FROM \"{}\"", spec.table);
    let rows = match client.query(&sql, &[]).await {
        Ok(rows) => rows,
        Err(source) => {
            return Err(ScanError::Read {
                location: spec.location.to_owned(),
                source,
            });
        }
    };
    let mut report = LocationReport {
        location: spec.location.to_owned(),
        scanned: 0,
        migrated: 0,
        already_v2: 0,
        plaintext: 0,
        undecryptable: 0,
        legacy: 0,
    };
    for row in &rows {
        for column in spec.json_columns {
            let Some(value) = row_value(row, column) else {
                continue;
            };
            if spec.table == "litellm_mcpservertable"
                && (*column == "static_headers" || *column == "env")
            {
                report.count(_classify_secret_map(&value, cipher, column))?;
            } else {
                for text in iter_encrypted_strings(&value) {
                    report.count(Some(classify_value(
                        &MigrationJson::String(text.clone()),
                        cipher,
                        "scan",
                    )))?;
                }
            }
        }
        for column in spec.scalar_columns {
            let Some(value) = row_value(row, column) else {
                continue;
            };
            if let MigrationJson::String(text) = value {
                let class = classify_value(&MigrationJson::String(text), cipher, column);
                report.count(Some(class))?;
            }
        }
    }
    Ok(report)
}

async fn _scan_config_env_vars<C: GenericClient + Sync>(
    client: &C,
    cipher: &DbCipher,
) -> Result<LocationReport, ScanError> {
    let mut report = LocationReport {
        location: String::from("config_environment_variables"),
        scanned: 0,
        migrated: 0,
        already_v2: 0,
        plaintext: 0,
        undecryptable: 0,
        legacy: 0,
    };
    let row = match client
        .query_opt(
            "SELECT \"param_value\" FROM \"litellm_config\" WHERE \"param_name\" = $1",
            &[&"environment_variables"],
        )
        .await
    {
        Ok(row) => row,
        Err(source) => {
            return Err(ScanError::Read {
                location: report.location.clone(),
                source,
            });
        }
    };
    if let Some(row) = row
        && let Some(value) = row_value(&row, "param_value")
    {
        for text in iter_encrypted_strings(&value) {
            report.count(Some(classify_value(
                &MigrationJson::String(text.clone()),
                cipher,
                "scan",
            )))?;
        }
    }
    Ok(report)
}

pub async fn _scan_covered_tables<C: GenericClient + Sync>(
    client: &C,
    cipher: &DbCipher,
) -> MigrationReport {
    let mut locations = Vec::new();
    for spec in COVERED_TABLE_SPECS {
        locations.push(
            _scan_one_table(client, spec, cipher)
                .await
                .unwrap_or_else(|_| LocationReport {
                    location: spec.location.to_owned(),
                    scanned: 0,
                    migrated: 0,
                    already_v2: 0,
                    plaintext: 0,
                    undecryptable: 0,
                    legacy: 0,
                }),
        );
    }
    locations.push(
        _scan_config_env_vars(client, cipher)
            .await
            .unwrap_or_else(|_| LocationReport {
                location: String::from("config_environment_variables"),
                scanned: 0,
                migrated: 0,
                already_v2: 0,
                plaintext: 0,
                undecryptable: 0,
                legacy: 0,
            }),
    );
    MigrationReport { locations }
}

mod tests;
