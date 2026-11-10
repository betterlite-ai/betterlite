#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::azure_oidc_exchange::{DEFAULT_AUTHORITY_HOST, Settings};

fn settings(variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .azure_oidc_exchange
}

fn authority(variables: &[(&str, &str)]) -> String {
    settings(variables).azure_authority_host
}

#[rstest]
#[case::loads_azure_authority_host(0)]
#[case::rejects_unrelated_key(1)]
fn azure_oidc_exchange_config_cases(#[case] case: u8) {
    if case == 0 {
        loads_azure_authority_host();
    } else {
        rejects_unrelated_key();
    }
}

fn loads_azure_authority_host() {
    assert_eq!(authority(&[]), DEFAULT_AUTHORITY_HOST);
    assert_eq!(
        authority(&[("AZURE_AUTHORITY_HOST", "https://login.contoso.test/")]),
        "https://login.contoso.test/"
    );
    assert_eq!(authority(&[("AZURE_AUTHORITY_HOST", "")]), "");
    assert_eq!(
        authority(&[("AZURE_AUTHORITY_HOST", "explicit"), ("UNRELATED", "value")]),
        "explicit"
    );
    assert_eq!(
        authority(&[("AZURE_AUTHORITY_HOST_OTHER", "https://other.test")]),
        DEFAULT_AUTHORITY_HOST
    );
}

fn rejects_unrelated_key() {
    let source = "litellm_settings:\n  azure_oidc_exchange_unrelated_key: 1\n";
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
        vec!["litellm_settings.azure_oidc_exchange_unrelated_key".to_owned()]
    );
}
