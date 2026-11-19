use std::collections::BTreeMap;

use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::DecryptPolicy;
use betterlite_secrets::secret_codec::FailureLog;
use betterlite_secrets::secret_codec::FailureValue;
use betterlite_secrets::secret_codec::StoredValue;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use crate::auxiliary_repository::repository::PrismaClient;
use crate::budget_permission_rows::JsonData;
use crate::resource_repository::config::ConfigError;
use crate::resource_repository::config::ConfigRepository;
use crate::resource_repository::config::ConfigStoredRow;
use crate::resource_repository::table::KeyFilter;
use crate::stored_cipher::DbCipher;

// LiteLLM's decrypt_value_helper with exception_type="error" and return_original_value=False
const DECRYPT: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Error,
    value: FailureValue::Nothing,
};

#[derive(Debug, thiserror::Error)]
pub enum SettingsRowError {
    #[error("the {param} config row cannot be read")]
    Read {
        param: &'static str,
        #[source]
        source: ConfigError,
    },
    #[error("the {param} config row cannot be written")]
    Write {
        param: &'static str,
        #[source]
        source: ConfigError,
    },
    #[error("the {param} config row does not hold a mapping")]
    Shape { param: &'static str },
    #[error("the {param} row field {field} is not a string")]
    Field {
        param: &'static str,
        field: &'static str,
    },
    #[error("the {field} credential cannot be encrypted")]
    Encrypt {
        field: &'static str,
        #[source]
        source: SecretError,
    },
    #[error("the {field} credential cannot be decrypted; check the salt key configuration")]
    Decrypt { field: &'static str },
}

pub async fn read_settings_row(
    client: Option<PrismaClient<'_>>,
    param: &'static str,
) -> Result<Option<BTreeMap<String, JsonData>>, SettingsRowError> {
    let stored = ConfigRepository::new(client, false)
        .get_param(param)
        .await
        .map_err(|source| SettingsRowError::Read { param, source })?;
    let Some(config) = stored else {
        return Ok(None);
    };
    match config.param_value {
        JsonData::Null => Ok(None),
        JsonData::Object(fields) => Ok(Some(fields)),
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Array(_) => {
            Err(SettingsRowError::Shape { param })
        }
    }
}

// LiteLLM checks the raw param_value for None without decoding it, so a malformed value still counts
pub async fn settings_row_present(
    client: Option<PrismaClient<'_>>,
    param: &'static str,
) -> Result<bool, SettingsRowError> {
    let rows = ConfigRepository::new(client, false)
        .table()
        .map_err(|source| SettingsRowError::Read {
            param,
            source: ConfigError::Database(source),
        })?
        .as_ref()
        .find::<ConfigStoredRow>(KeyFilter::Equal(param))
        .await
        .map_err(|source| SettingsRowError::Read {
            param,
            source: ConfigError::Table(source),
        })?;
    Ok(rows.iter().any(|row| row.param_value != JsonData::Null))
}

// LiteLLM stores json.dumps(settings), which the table keeps as a JSONB string
pub async fn write_settings_row(
    client: Option<PrismaClient<'_>>,
    param: &'static str,
    fields: BTreeMap<String, JsonData>,
) -> Result<(), SettingsRowError> {
    ConfigRepository::new(client, false)
        .set_param(param, JsonData::Object(fields))
        .await
        .map_err(|source| SettingsRowError::Write { param, source })?;
    Ok(())
}

pub fn plain_field(
    fields: &BTreeMap<String, JsonData>,
    param: &'static str,
    field: &'static str,
) -> Result<Option<String>, SettingsRowError> {
    match fields.get(field) {
        None | Some(JsonData::Null) => Ok(None),
        Some(JsonData::String(text)) => Ok(Some(text.clone())),
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
        ) => Err(SettingsRowError::Field { param, field }),
    }
}

pub fn secret_field(
    fields: &BTreeMap<String, JsonData>,
    param: &'static str,
    field: &'static str,
    key: &'static str,
    cipher: &DbCipher,
) -> Result<Option<SecretString>, SettingsRowError> {
    let Some(text) = plain_field(fields, param, field)? else {
        return Ok(None);
    };
    // LiteLLM's `if encrypted_api_key:` skips an empty value, so it never reaches the cipher
    if text.is_empty() {
        return Ok(Some(SecretString::default()));
    }
    let stored = StoredValue::<()>::Text(SecretString::from(text));
    let Some(StoredValue::Text(plaintext)) = cipher.decrypt_value(stored, key, DECRYPT) else {
        return Err(SettingsRowError::Decrypt { field: key });
    };
    Ok(Some(plaintext))
}

pub fn seal_secret(
    cipher: &DbCipher,
    key: &'static str,
    value: &SecretString,
) -> Result<String, SettingsRowError> {
    let stored = StoredValue::<()>::Text(value.clone());
    match cipher.encrypt_value(stored, None) {
        Ok(StoredValue::Text(sealed)) => Ok(sealed.expose_secret().to_owned()),
        Ok(StoredValue::Other(())) => Ok(value.expose_secret().to_owned()),
        Err(source) => Err(SettingsRowError::Encrypt { field: key, source }),
    }
}
