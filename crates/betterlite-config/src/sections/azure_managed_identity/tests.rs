#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::azure_managed_identity::Settings;

#[derive(Clone, Copy)]
enum Expect {
    Loads,
    LoadsSecret,
    Rejected,
}

fn settings(variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .azure_managed_identity
}

#[rstest]
#[case::loads_identity_endpoint("IDENTITY_ENDPOINT", "http://sf.test/token", |settings: &Settings| settings.identity_endpoint.clone(), Expect::Loads)]
#[case::loads_identity_header("IDENTITY_HEADER", "fixture-header-secret", |settings: &Settings| settings.identity_header.as_ref().map(|secret| secret.expose_secret().to_owned()), Expect::LoadsSecret)]
#[case::loads_identity_server_thumbprint("IDENTITY_SERVER_THUMBPRINT", "00112233445566778899aabbccddeeff00112233", |settings: &Settings| settings.identity_server_thumbprint.clone(), Expect::Loads)]
#[case::loads_imds_endpoint("IMDS_ENDPOINT", "http://arc.test/imds", |settings: &Settings| settings.imds_endpoint.clone(), Expect::Loads)]
#[case::loads_msi_endpoint("MSI_ENDPOINT", "http://ml.test/token", |settings: &Settings| settings.msi_endpoint.clone(), Expect::Loads)]
#[case::loads_msi_secret("MSI_SECRET", "fixture-msi-secret", |settings: &Settings| settings.msi_secret.as_ref().map(|secret| secret.expose_secret().to_owned()), Expect::LoadsSecret)]
#[case::loads_azure_pod_identity_authority_host("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://169.254.170.2", |settings: &Settings| settings.pod_identity_authority_host.clone(), Expect::Loads)]
#[case::rejects_unrelated_key("UNRELATED_IDENTITY_ENDPOINT", "http://elsewhere.test/token", |settings: &Settings| settings.identity_endpoint.clone(), Expect::Rejected)]
fn azure_managed_identity_config_cases(
    #[case] name: &str,
    #[case] value: &str,
    #[case] read: fn(&Settings) -> Option<String>,
    #[case] expect: Expect,
) {
    let secret = match expect {
        Expect::Rejected => {
            assert_eq!(read(&settings(&[(name, value)])), None);
            rejects_unrelated_key();
            return;
        }
        Expect::LoadsSecret => true,
        Expect::Loads => false,
    };
    let explicit = settings(&[(name, value)]);
    assert_eq!(read(&explicit).as_deref(), Some(value));
    assert_eq!(read(&settings(&[])), None);
    assert_eq!(read(&settings(&[(name, "")])).as_deref(), Some(""));
    assert_eq!(read(&settings(&[("UNRELATED", value)])), None);
    if secret {
        assert!(!format!("{explicit:?}").contains(value));
        assert_eq!(
            explicit.redact_known(&format!("echo {value} back")),
            "echo [redacted] back"
        );
    }
}

fn rejects_unrelated_key() {
    let source = "litellm_settings:\n  azure_managed_identity_unrelated_key: 1\n";
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unrelated key loaded without an unsupported-key error");
    };
    assert_eq!(
        paths,
        vec!["litellm_settings.azure_managed_identity_unrelated_key".to_owned()]
    );
}
