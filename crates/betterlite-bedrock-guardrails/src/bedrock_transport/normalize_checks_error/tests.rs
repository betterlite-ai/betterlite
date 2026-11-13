#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::bedrock_transport::BedrockGuardrailError;
use crate::guardrail_checks::{_normalize_checks, ChecksInput};

#[test]
fn normalize_checks_error_when_only_unknown_keys() {
    let input = BTreeMap::from([("unknownCheck".to_owned(), GuardrailValue::Boolean(true))]);
    let error =
        _normalize_checks(Some(ChecksInput::Mapping(input))).expect_err("only unknown keys");
    match error {
        BedrockGuardrailError::NoUsableChecks { keys } => {
            assert_eq!(keys, vec!["unknownCheck".to_owned()]);
        }
        other => panic!("expected the no-usable-checks error, got {other}"),
    }
}

#[test]
fn normalize_checks_error_when_known_keys_are_all_null() {
    let input = BTreeMap::from([("sensitiveInformation".to_owned(), GuardrailValue::Null)]);
    let error = _normalize_checks(Some(ChecksInput::Mapping(input))).expect_err("only null keys");
    match error {
        BedrockGuardrailError::NoUsableChecks { keys } => {
            assert_eq!(keys, vec!["sensitiveInformation".to_owned()]);
        }
        other => panic!("expected the no-usable-checks error, got {other}"),
    }
}
