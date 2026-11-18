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
    assert_eq!(omitted.limit_for_index(0), None);

    let null =
        load("model_list:\n  - model_name: a\n    litellm_params:\n      max_file_size_mb: null\n")
            .unwrap();
    assert_eq!(null.limit_for_index(0), None);

    let configured =
        load("model_list:\n  - model_name: a\n    litellm_params:\n      max_file_size_mb: 1\n")
            .unwrap();
    assert_eq!(configured.limit_for_index(0), Some(1));
}
