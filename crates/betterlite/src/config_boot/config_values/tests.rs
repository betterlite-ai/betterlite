#![cfg(test)]

use betterlite_config::credentials::CredentialValue;

use super::{credential_list, misplaced_jwt_keys};
use crate::config_boot::test_support::{capture_logs, config_from_yaml};

#[test]
fn no_misplaced_jwt_keys_report_nothing() {
    let config = config_from_yaml("general_settings:\n  enable_jwt_auth: true\n");
    let (logs, _guard) = capture_logs();
    assert!(misplaced_jwt_keys(&config).is_empty());
    assert!(!logs.contents().contains("Ignoring top-level config key(s)"));
}

#[test]
fn both_jwt_keys_are_reported_in_fixed_order() {
    let config = config_from_yaml("litellm_jwtauth:\n  a: 1\nenable_jwt_auth: true\n");
    let (logs, _guard) = capture_logs();
    assert_eq!(
        misplaced_jwt_keys(&config),
        vec!["enable_jwt_auth", "litellm_jwtauth"]
    );
    assert!(
        logs.contents()
            .contains("Ignoring top-level config key(s) enable_jwt_auth, litellm_jwtauth.")
    );
}

#[test]
fn a_single_key_is_named_alone_in_the_warning() {
    let config = config_from_yaml("litellm_jwtauth:\n  a: 1\n");
    let (logs, _guard) = capture_logs();
    assert_eq!(misplaced_jwt_keys(&config), vec!["litellm_jwtauth"]);
    assert!(
        logs.contents()
            .contains("Ignoring top-level config key(s) litellm_jwtauth.")
    );
}

#[test]
fn a_null_jwt_key_still_counts_as_misplaced() {
    let config = config_from_yaml("enable_jwt_auth: null\n");
    let (_logs, _guard) = capture_logs();
    assert_eq!(misplaced_jwt_keys(&config), vec!["enable_jwt_auth"]);
}

#[test]
fn a_missing_credential_list_is_empty() {
    let config = config_from_yaml("model_list:\n  - model_name: gpt\n");
    assert!(credential_list(&config).unwrap().is_empty());
}

#[test]
fn a_null_credential_list_is_empty() {
    let config = config_from_yaml("credential_list:\n");
    assert!(credential_list(&config).unwrap().is_empty());
}

#[test]
fn an_empty_credential_list_is_empty() {
    let config = config_from_yaml("credential_list: []\n");
    assert!(credential_list(&config).unwrap().is_empty());
}

#[test]
fn credential_items_are_parsed_with_their_values() {
    let config = config_from_yaml(
        "credential_list:\n  - credential_name: primary\n    credential_info:\n      custom_llm_provider: openai\n    credential_values:\n      api_key: sk-test\n",
    );
    let list = credential_list(&config).unwrap();
    assert_eq!(list.len(), 1);
    let item = list.first().unwrap();
    assert_eq!(item.credential_name, "primary");
    assert!(item.credential_info.0.contains_key("custom_llm_provider"));
    assert!(matches!(
        item.credential_values.0.get("api_key"),
        Some(CredentialValue::Text(_))
    ));
}

#[test]
fn a_credential_list_of_another_shape_is_refused() {
    let config = config_from_yaml("credential_list: hello\n");
    let error = credential_list(&config).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("credential_list is not a list of credential items")
    );
}
