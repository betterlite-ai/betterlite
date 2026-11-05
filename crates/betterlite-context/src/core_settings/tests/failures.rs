#![cfg(test)]

use super::{BTreeMap, ConfigError, ConfigLoader};
use rstest::rstest;

#[rstest]
#[case::unsupported_module_setting(
    "litellm_settings:\n  unsupported_timeout: 2\n",
    "litellm_settings.unsupported_timeout"
)]
#[case::unsupported_model_setting(
    "model_list:\n  - model_name: x\n    litellm_params:\n      unsupported_timeout: 2\n",
    "model_list.0.litellm_params.unsupported_timeout"
)]
fn call_context_tests_behavior(#[case] yaml: &str, #[case] path: &str) {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let ConfigError::Unsupported { paths } = loader.load(yaml).unwrap_err() else {
        panic!("expected an unsupported-key error");
    };
    assert!(paths.contains(&path.to_owned()));
}

mod malformed;
