use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

pub const ACCOUNT_NAME_VARIABLE: &str = "AZURE_STORAGE_ACCOUNT_NAME";
pub const FILE_SYSTEM_VARIABLE: &str = "AZURE_STORAGE_FILE_SYSTEM";
pub const TENANT_ID_VARIABLE: &str = "AZURE_STORAGE_TENANT_ID";
pub const CLIENT_ID_VARIABLE: &str = "AZURE_STORAGE_CLIENT_ID";
pub const CLIENT_SECRET_VARIABLE: &str = "AZURE_STORAGE_CLIENT_SECRET";
pub const ACCOUNT_KEY_VARIABLE: &str = "AZURE_STORAGE_ACCOUNT_KEY";
pub const ENDPOINT_SUFFIX_VARIABLE: &str = "AZURE_STORAGE_ENDPOINT_SUFFIX";
pub const DEFAULT_ENDPOINT_SUFFIX: &str = "core.windows.net";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("Missing required environment variable: {variable}")]
    Missing { variable: &'static str },
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub account_name: String,
    pub file_system: String,
    pub endpoint_suffix: String,
    pub dfs_endpoint: String,
    pub blob_endpoint: String,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub account_key: Option<SecretString>,
}

impl Settings {
    pub fn from_environment(environment: &dyn EnvironmentLookup) -> Result<Self, SettingsError> {
        let account_name = required(environment, ACCOUNT_NAME_VARIABLE)?;
        let file_system = required(environment, FILE_SYSTEM_VARIABLE)?;
        let endpoint_suffix = optional(environment, ENDPOINT_SUFFIX_VARIABLE)
            .unwrap_or_else(|| DEFAULT_ENDPOINT_SUFFIX.to_owned());
        let dfs_endpoint = format!("https://{account_name}.dfs.{endpoint_suffix}");
        let blob_endpoint = format!("https://{account_name}.blob.{endpoint_suffix}");
        Ok(Self {
            account_name,
            file_system,
            endpoint_suffix,
            dfs_endpoint,
            blob_endpoint,
            tenant_id: optional(environment, TENANT_ID_VARIABLE),
            client_id: optional(environment, CLIENT_ID_VARIABLE),
            client_secret: optional(environment, CLIENT_SECRET_VARIABLE).map(SecretString::from),
            account_key: optional(environment, ACCOUNT_KEY_VARIABLE).map(SecretString::from),
        })
    }
}

fn required(
    environment: &dyn EnvironmentLookup,
    variable: &'static str,
) -> Result<String, SettingsError> {
    optional(environment, variable).ok_or(SettingsError::Missing { variable })
}

fn optional(environment: &dyn EnvironmentLookup, variable: &str) -> Option<String> {
    environment
        .lookup(variable)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests;
