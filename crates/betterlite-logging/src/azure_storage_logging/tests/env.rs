#![cfg(test)]

use super::support::{ACCOUNT, ENTRA, environment};
use crate::azure_storage_logging::settings::AzureStorageSettings;

fn settings(variables: &[(&str, &str)]) -> AzureStorageSettings {
    AzureStorageSettings::from_environment(&environment(variables)).expect("valid settings")
}

#[test]
fn the_optional_credentials_default_to_absent() {
    let settings = settings(&ACCOUNT);
    assert!(settings.tenant_id.is_none());
    assert!(settings.client_id.is_none());
    assert!(settings.client_secret.is_none());
    assert!(settings.account_key.is_some());
}

#[test]
fn an_empty_optional_variable_counts_as_unset() {
    let variables = [
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
        ("AZURE_STORAGE_TENANT_ID", ""),
        ("AZURE_STORAGE_CLIENT_ID", ""),
        ("AZURE_STORAGE_CLIENT_SECRET", ""),
        ("AZURE_STORAGE_ACCOUNT_KEY", ""),
        ("AZURE_STORAGE_ENDPOINT_SUFFIX", ""),
    ];
    let settings = settings(&variables);
    assert!(settings.tenant_id.is_none());
    assert!(settings.client_id.is_none());
    assert!(settings.client_secret.is_none());
    assert!(settings.account_key.is_none());
    assert_eq!(settings.endpoint_suffix, "core.windows.net");
}

#[test]
fn the_endpoint_suffix_builds_both_endpoints() {
    let variables = [
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
        ("AZURE_STORAGE_ENDPOINT_SUFFIX", "core.chinacloudapi.cn"),
    ];
    let settings = settings(&variables);
    assert_eq!(
        settings.dfs_endpoint(),
        "https://acct.dfs.core.chinacloudapi.cn"
    );
    assert_eq!(
        settings.blob_endpoint(),
        "https://acct.blob.core.chinacloudapi.cn"
    );
}

#[test]
fn the_service_principal_values_are_read_and_kept_secret() {
    let settings = settings(&ENTRA);
    assert_eq!(settings.tenant_id.as_deref(), Some("tenant-1"));
    assert_eq!(settings.client_id.as_deref(), Some("client-1"));
    assert_eq!(
        settings
            .client_secret
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("secret-1")
    );
    assert!(settings.account_key.is_none());
}

#[test]
fn the_account_key_is_read() {
    let settings = settings(&ACCOUNT);
    assert_eq!(
        settings
            .account_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("a2V5")
    );
}
