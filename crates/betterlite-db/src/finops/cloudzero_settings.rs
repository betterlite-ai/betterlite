use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::auxiliary_repository::repository::PrismaClient;
use crate::budget_permission_rows::JsonData;
use crate::stored_cipher::DbCipher;

use super::settings_row::SettingsRowError;
use super::settings_row::plain_field;
use super::settings_row::read_settings_row;
use super::settings_row::seal_secret;
use super::settings_row::secret_field;
use super::settings_row::write_settings_row;

pub const CLOUDZERO_SETTINGS_PARAM_NAME: &str = "cloudzero_settings";
pub const CLOUDZERO_API_KEY: &str = "cloudzero_api_key";

#[derive(Debug, Clone)]
pub struct CloudZeroSettings {
    pub api_key: Option<SecretString>,
    pub connection_id: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CloudZeroSettingsWrite {
    pub api_key: SecretString,
    pub connection_id: String,
    pub timezone: String,
}

pub async fn get_cloudzero_settings(
    client: Option<PrismaClient<'_>>,
    cipher: &DbCipher,
) -> Result<Option<CloudZeroSettings>, SettingsRowError> {
    let row = read_settings_row(client, CLOUDZERO_SETTINGS_PARAM_NAME).await?;
    let Some(fields) = row else {
        return Ok(None);
    };
    let api_key = secret_field(
        &fields,
        CLOUDZERO_SETTINGS_PARAM_NAME,
        "api_key",
        CLOUDZERO_API_KEY,
        cipher,
    )?;
    Ok(Some(CloudZeroSettings {
        api_key,
        connection_id: plain_field(&fields, CLOUDZERO_SETTINGS_PARAM_NAME, "connection_id")?,
        timezone: plain_field(&fields, CLOUDZERO_SETTINGS_PARAM_NAME, "timezone")?,
    }))
}

pub async fn set_cloudzero_settings(
    client: Option<PrismaClient<'_>>,
    cipher: &DbCipher,
    settings: &CloudZeroSettingsWrite,
) -> Result<(), SettingsRowError> {
    let sealed_api_key = seal_secret(cipher, CLOUDZERO_API_KEY, &settings.api_key)?;
    let mut fields = BTreeMap::new();
    fields.insert("api_key".to_owned(), JsonData::String(sealed_api_key));
    fields.insert(
        "connection_id".to_owned(),
        JsonData::String(settings.connection_id.clone()),
    );
    fields.insert(
        "timezone".to_owned(),
        JsonData::String(settings.timezone.clone()),
    );
    write_settings_row(client, CLOUDZERO_SETTINGS_PARAM_NAME, fields).await
}
