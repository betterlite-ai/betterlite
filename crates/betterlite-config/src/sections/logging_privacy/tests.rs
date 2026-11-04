#![cfg(test)]

use crate::loader::ConfigLoader;
use rstest::rstest;
use std::collections::BTreeMap;

#[rstest]
#[case::defaults("{}", "", Some((false, false)))]
#[case::enabled("litellm_settings: {turn_off_message_logging: true, redact_user_api_key_info: true}", "", Some((true, true)))]
#[case::explicit_false("litellm_settings: {turn_off_message_logging: false, redact_user_api_key_info: false}", "", Some((false, false)))]
#[case::null_defaults("litellm_settings: {turn_off_message_logging: null, redact_user_api_key_info: null}", "", Some((false, false)))]
#[case::environment("litellm_settings: {turn_off_message_logging: os.environ/FLAG, redact_user_api_key_info: os.environ/FLAG}", "true", Some((true, true)))]
#[case::empty_environment("litellm_settings: {turn_off_message_logging: os.environ/FLAG}", "", Some((false, false)))]
#[case::missing_environment(
    "litellm_settings: {turn_off_message_logging: os.environ/ABSENT}",
    "",
    None
)]
#[case::invalid_environment(
    "litellm_settings: {redact_user_api_key_info: os.environ/FLAG}",
    "invalid",
    None
)]
#[case::invalid_type("litellm_settings: {turn_off_message_logging: 1}", "", None)]
fn logging_privacy_tests_behavior(
    #[case] yaml: &str,
    #[case] flag: &str,
    #[case] expected: Option<(bool, bool)>,
) {
    let result = ConfigLoader {
        lookup: BTreeMap::from([("FLAG".to_owned(), flag.to_owned())]),
    }
    .load(yaml);
    match expected {
        Some(flags) => {
            let settings = result.unwrap().sections.logging_privacy;
            assert_eq!(
                (
                    settings.turn_off_message_logging,
                    settings.redact_user_api_key_info
                ),
                flags
            );
        }
        None => assert!(result.is_err()),
    }
}
