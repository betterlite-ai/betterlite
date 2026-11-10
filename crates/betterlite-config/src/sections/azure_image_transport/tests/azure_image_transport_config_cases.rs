#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::azure_image_transport::{DEFAULT_POLLING_TIMEOUT, Settings};

const TIMEOUT_ENV: &str = "AZURE_OPERATION_POLLING_TIMEOUT";

#[rstest]
#[case::loads_azure_operation_polling_timeout(loads_azure_operation_polling_timeout)]
#[case::rejects_unrelated_key(rejects_unrelated_key)]
fn azure_image_transport_config_cases(#[case] scenario: fn()) {
    scenario();
}

fn settings(lookup: BTreeMap<String, String>) -> Settings {
    ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .azure_image_transport
}

fn loads_azure_operation_polling_timeout() {
    assert_eq!(
        settings(BTreeMap::new()).operation_polling_timeout,
        DEFAULT_POLLING_TIMEOUT
    );
    let configured = BTreeMap::from([(TIMEOUT_ENV.to_owned(), "45".to_owned())]);
    assert_eq!(
        settings(configured).operation_polling_timeout,
        Duration::from_secs(45)
    );
    let blank = BTreeMap::from([(TIMEOUT_ENV.to_owned(), String::new())]);
    assert_eq!(
        settings(blank).operation_polling_timeout,
        Duration::from_secs(120)
    );
    let invalid = BTreeMap::from([(TIMEOUT_ENV.to_owned(), "soon".to_owned())]);
    let error = ConfigLoader { lookup: invalid }.load("").unwrap_err();
    assert!(error.to_string().contains(TIMEOUT_ENV));
}

fn rejects_unrelated_key() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings: {azure_image_transport_unrelated: value}")
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("litellm_settings.azure_image_transport_unrelated")
    );
}
