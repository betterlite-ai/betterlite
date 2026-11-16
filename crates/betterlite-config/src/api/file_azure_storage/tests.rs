use std::collections::BTreeMap;

use rstest::rstest;

use super::{DEFAULT_ENDPOINT_SUFFIX, Settings, SettingsError};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn required_environment() -> Vec<(&'static str, &'static str)> {
    vec![
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "files"),
    ]
}

mod loads_settings {
    use super::*;

    #[test]
    fn reads_the_required_variables_and_defaults_the_suffix() {
        let settings =
            Settings::from_environment(&environment(&required_environment())).expect("settings");
        assert_eq!(settings.account_name, "acct");
        assert_eq!(settings.file_system, "files");
        assert_eq!(settings.endpoint_suffix, DEFAULT_ENDPOINT_SUFFIX);
        assert_eq!(settings.dfs_endpoint, "https://acct.dfs.core.windows.net");
        assert_eq!(settings.blob_endpoint, "https://acct.blob.core.windows.net");
    }

    #[test]
    fn reads_every_optional_variable() {
        let mut pairs = required_environment();
        pairs.extend([
            ("AZURE_STORAGE_TENANT_ID", "tenant"),
            ("AZURE_STORAGE_CLIENT_ID", "client"),
            ("AZURE_STORAGE_CLIENT_SECRET", "secret"),
            ("AZURE_STORAGE_ACCOUNT_KEY", "a2V5"),
            ("AZURE_STORAGE_ENDPOINT_SUFFIX", "core.usgovcloudapi.net"),
        ]);
        let settings = Settings::from_environment(&environment(&pairs)).expect("settings");
        assert_eq!(settings.tenant_id.as_deref(), Some("tenant"));
        assert_eq!(settings.client_id.as_deref(), Some("client"));
        assert_eq!(
            settings
                .client_secret
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret),
            Some("secret")
        );
        assert_eq!(
            settings
                .account_key
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret),
            Some("a2V5")
        );
        assert_eq!(settings.endpoint_suffix, "core.usgovcloudapi.net");
        assert_eq!(
            settings.dfs_endpoint,
            "https://acct.dfs.core.usgovcloudapi.net"
        );
        assert_eq!(
            settings.blob_endpoint,
            "https://acct.blob.core.usgovcloudapi.net"
        );
    }

    #[test]
    fn an_empty_suffix_falls_back_to_the_default() {
        let mut pairs = required_environment();
        pairs.push(("AZURE_STORAGE_ENDPOINT_SUFFIX", ""));
        let settings = Settings::from_environment(&environment(&pairs)).expect("settings");
        assert_eq!(settings.endpoint_suffix, DEFAULT_ENDPOINT_SUFFIX);
        assert_eq!(settings.dfs_endpoint, "https://acct.dfs.core.windows.net");
    }

    #[test]
    fn empty_optional_variables_are_unset() {
        let mut pairs = required_environment();
        pairs.extend([
            ("AZURE_STORAGE_TENANT_ID", ""),
            ("AZURE_STORAGE_CLIENT_ID", ""),
            ("AZURE_STORAGE_CLIENT_SECRET", ""),
            ("AZURE_STORAGE_ACCOUNT_KEY", ""),
        ]);
        let settings = Settings::from_environment(&environment(&pairs)).expect("settings");
        assert!(settings.tenant_id.is_none());
        assert!(settings.client_id.is_none());
        assert!(settings.client_secret.is_none());
        assert!(settings.account_key.is_none());
    }

    #[rstest]
    #[case::missing_account_name(
        vec![("AZURE_STORAGE_FILE_SYSTEM", "files")],
        "AZURE_STORAGE_ACCOUNT_NAME"
    )]
    #[case::missing_file_system(
        vec![("AZURE_STORAGE_ACCOUNT_NAME", "acct")],
        "AZURE_STORAGE_FILE_SYSTEM"
    )]
    #[case::empty_account_name(
        vec![
            ("AZURE_STORAGE_ACCOUNT_NAME", ""),
            ("AZURE_STORAGE_FILE_SYSTEM", "files"),
        ],
        "AZURE_STORAGE_ACCOUNT_NAME"
    )]
    #[case::empty_file_system(
        vec![
            ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
            ("AZURE_STORAGE_FILE_SYSTEM", ""),
        ],
        "AZURE_STORAGE_FILE_SYSTEM"
    )]
    fn missing_required_variables_name_the_variable(
        #[case] pairs: Vec<(&'static str, &'static str)>,
        #[case] variable: &'static str,
    ) {
        let error = Settings::from_environment(&environment(&pairs)).expect_err("missing");
        assert_eq!(error, SettingsError::Missing { variable });
        assert_eq!(
            error.to_string(),
            format!("Missing required environment variable: {variable}")
        );
    }

    #[test]
    fn accepts_the_environment_map_the_process_loader_builds() {
        let process_map: BTreeMap<String, String> = environment(&required_environment());
        let settings = Settings::from_environment(&process_map).expect("settings");
        assert_eq!(settings.account_name, "acct");
    }
}
