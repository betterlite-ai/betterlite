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

pub const VANTAGE_SETTINGS_PARAM_NAME: &str = "vantage_settings";
pub const VANTAGE_API_KEY: &str = "vantage_api_key";
pub const VANTAGE_INTEGRATION_TOKEN: &str = "vantage_integration_token";

#[derive(Debug, Clone)]
pub struct VantageSettings {
    pub api_key: Option<SecretString>,
    pub integration_token: Option<SecretString>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VantageSettingsWrite {
    pub api_key: SecretString,
    pub integration_token: SecretString,
    pub base_url: String,
}

pub async fn get_vantage_settings(
    client: Option<PrismaClient<'_>>,
    cipher: &DbCipher,
) -> Result<Option<VantageSettings>, SettingsRowError> {
    let Some(fields) = read_settings_row(client, VANTAGE_SETTINGS_PARAM_NAME).await? else {
        return Ok(None);
    };
    let base_url = plain_field(&fields, VANTAGE_SETTINGS_PARAM_NAME, "base_url")?;
    Ok(Some(VantageSettings {
        api_key: secret_field(
            &fields,
            VANTAGE_SETTINGS_PARAM_NAME,
            "api_key",
            VANTAGE_API_KEY,
            cipher,
        )?,
        integration_token: secret_field(
            &fields,
            VANTAGE_SETTINGS_PARAM_NAME,
            "integration_token",
            VANTAGE_INTEGRATION_TOKEN,
            cipher,
        )?,
        base_url,
    }))
}

pub async fn set_vantage_settings(
    client: Option<PrismaClient<'_>>,
    cipher: &DbCipher,
    settings: &VantageSettingsWrite,
) -> Result<(), SettingsRowError> {
    let mut fields = BTreeMap::new();
    fields.insert(
        "api_key".to_owned(),
        JsonData::String(seal_secret(cipher, VANTAGE_API_KEY, &settings.api_key)?),
    );
    fields.insert(
        "integration_token".to_owned(),
        JsonData::String(seal_secret(
            cipher,
            VANTAGE_INTEGRATION_TOKEN,
            &settings.integration_token,
        )?),
    );
    fields.insert(
        "base_url".to_owned(),
        JsonData::String(settings.base_url.clone()),
    );
    write_settings_row(client, VANTAGE_SETTINGS_PARAM_NAME, fields).await
}
