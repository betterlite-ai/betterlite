#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::request_form::Settings;

fn load(source: &str) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map(|config| config.sections.request_form)
}

#[test]
fn loads_settings() {
    let omitted = load("model_list:\n  - model_name: a\n    litellm_params: {}\n").unwrap();
    assert_eq!(omitted.deployments[0], None);

    let null =
        load("model_list:\n  - model_name: a\n    litellm_params:\n      max_file_size_mb: null\n")
            .unwrap();
    assert_eq!(null.deployments[0], None);

    let configured =
        load("model_list:\n  - model_name: a\n    litellm_params:\n      max_file_size_mb: 1\n")
            .unwrap();
    assert_eq!(configured.deployments[0], Some(1.0));

    let fractional =
        load("model_list:\n  - model_name: a\n    litellm_params:\n      max_file_size_mb: 0.5\n")
            .unwrap();
    assert_eq!(fractional.deployments[0], Some(0.5));
}

#[test]
fn the_configured_key_is_not_reported_as_unsupported() {
    let result =
        load("model_list:\n  - model_name: a\n    litellm_params:\n      max_file_size_mb: 1\n");
    assert!(!matches!(&result, Err(ConfigError::Unsupported { .. })));
    assert!(result.is_ok());
}

#[test]
fn a_remaining_unknown_key_is_still_reported() {
    let result =
        load("model_list:\n  - model_name: a\n    litellm_params:\n      unknown_form_key: 1\n");
    assert!(matches!(result, Err(ConfigError::Unsupported { .. })));
}
