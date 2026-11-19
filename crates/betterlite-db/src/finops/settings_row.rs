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

fn settings_fields(
    value: JsonData,
    param: &'static str,
) -> Result<Option<BTreeMap<String, JsonData>>, SettingsRowError> {
    match value {
        JsonData::Null => Ok(None),
        JsonData::Object(fields) => Ok(Some(fields)),
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Array(_) => {
            Err(SettingsRowError::Shape { param })
        }
    }
}

pub async fn read_settings_row(
    client: Option<PrismaClient<'_>>,
    param: &'static str,
) -> Result<Option<BTreeMap<String, JsonData>>, SettingsRowError> {
    let read_error = |source| SettingsRowError::Read { param, source };
    let stored = ConfigRepository::new(client, false)
        .get_param(param)
        .await
        .map_err(read_error)?;
    let Some(config) = stored else {
        return Ok(None);
    };
    settings_fields(config.param_value, param)
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
    let Some(value) = fields.get(field) else {
        return Ok(None);
    };
    let JsonData::String(text) = value else {
        if value == &JsonData::Null {
            return Ok(None);
        }
        return Err(SettingsRowError::Field { param, field });
    };
    Ok(Some(text.clone()))
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
