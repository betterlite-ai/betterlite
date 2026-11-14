use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

use crate::batch::logger::{
    BATCH_SIZE_VARIABLE, BatchSettings, FLUSH_INTERVAL_VARIABLE,
    SettingsError as BatchSettingsError,
};

pub const ACCOUNT_NAME_VARIABLE: &str = "AZURE_STORAGE_ACCOUNT_NAME";
pub const FILE_SYSTEM_VARIABLE: &str = "AZURE_STORAGE_FILE_SYSTEM";
pub const TENANT_ID_VARIABLE: &str = "AZURE_STORAGE_TENANT_ID";
pub const CLIENT_ID_VARIABLE: &str = "AZURE_STORAGE_CLIENT_ID";
pub const CLIENT_SECRET_VARIABLE: &str = "AZURE_STORAGE_CLIENT_SECRET";
pub const ACCOUNT_KEY_VARIABLE: &str = "AZURE_STORAGE_ACCOUNT_KEY";
pub const ENDPOINT_SUFFIX_VARIABLE: &str = "AZURE_STORAGE_ENDPOINT_SUFFIX";
pub const DEFAULT_ENDPOINT_SUFFIX: &str = "core.windows.net";
const BATCH_VARIABLES: [&str; 2] = [BATCH_SIZE_VARIABLE, FLUSH_INTERVAL_VARIABLE];

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("Missing required environment variable: AZURE_STORAGE_ACCOUNT_NAME")]
    MissingAccountName,
    #[error("Missing required environment variable: AZURE_STORAGE_FILE_SYSTEM")]
    MissingFileSystem,
    #[error("the Azure storage batch settings are not valid")]
    Batch(#[from] BatchSettingsError),
}

#[derive(Clone, Debug)]
pub struct AzureStorageSettings {
    pub account_name: String,
    pub file_system: String,
    pub endpoint_suffix: String,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub account_key: Option<SecretString>,
    pub batch: BatchSettings,
}

fn batch_variables(environment: &dyn EnvironmentLookup) -> BTreeMap<String, String> {
    let mut variables = BTreeMap::new();
    for name in BATCH_VARIABLES {
        if let Some(value) = environment.lookup(name) {
            let _previous = variables.insert(name.to_owned(), value);
        }
    }
    variables
}

impl AzureStorageSettings {
    pub fn from_environment(environment: &dyn EnvironmentLookup) -> Result<Self, SettingsError> {
        let account_name = environment
            .lookup(ACCOUNT_NAME_VARIABLE)
            .filter(|value| !value.is_empty())
            .ok_or(SettingsError::MissingAccountName)?;
        let file_system = environment
            .lookup(FILE_SYSTEM_VARIABLE)
            .filter(|value| !value.is_empty())
            .ok_or(SettingsError::MissingFileSystem)?;
        let endpoint_suffix = environment
            .lookup(ENDPOINT_SUFFIX_VARIABLE)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_ENDPOINT_SUFFIX.to_owned());
        Ok(Self {
            account_name,
            file_system,
            endpoint_suffix,
            tenant_id: environment
                .lookup(TENANT_ID_VARIABLE)
                .filter(|value| !value.is_empty()),
            client_id: environment
                .lookup(CLIENT_ID_VARIABLE)
                .filter(|value| !value.is_empty()),
            client_secret: environment
                .lookup(CLIENT_SECRET_VARIABLE)
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            account_key: environment
                .lookup(ACCOUNT_KEY_VARIABLE)
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            batch: BatchSettings::from_environment(&batch_variables(environment))?,
        })
    }

    #[must_use]
    pub fn dfs_endpoint(&self) -> String {
        format!("https://{}.dfs.{}", self.account_name, self.endpoint_suffix)
    }

    #[must_use]
    pub fn blob_endpoint(&self) -> String {
        format!(
            "https://{}.blob.{}",
            self.account_name, self.endpoint_suffix
        )
    }
}
