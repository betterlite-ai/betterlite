#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::bedrock_transport::BedrockGuardrailError;
use crate::guardrail_checks::{ChecksInput, normalize_checks};

#[test]
fn normalize_checks_error_when_only_unknown_keys() {
    let input = BTreeMap::from([("unknownCheck".to_owned(), GuardrailValue::Boolean(true))]);
    let error = normalize_checks(Some(ChecksInput::Mapping(input))).expect_err("only unknown keys");
    assert_eq!(
        error.to_string(),
        "BedrockGuardrail: 'checks' block contained only unrecognized or empty keys [\"unknownCheck\"]. Known keys: [\"contentFilter\", \"promptAttack\", \"sensitiveInformation\"]. Fix the guardrail config or remove the 'checks' block to use ApplyGuardrail mode."
    );
    let BedrockGuardrailError::NoUsableChecks { keys } = error else {
        panic!("expected the no-usable-checks error, got {error}");
    };
    assert_eq!(keys, vec!["unknownCheck".to_owned()]);
}

#[test]
fn normalize_checks_error_when_known_keys_are_all_null() {
    let input = BTreeMap::from([("sensitiveInformation".to_owned(), GuardrailValue::Null)]);
    let error = normalize_checks(Some(ChecksInput::Mapping(input))).expect_err("only null keys");
    let BedrockGuardrailError::NoUsableChecks { keys } = error else {
        panic!("expected the no-usable-checks error, got {error}");
    };
    assert_eq!(keys, vec!["sensitiveInformation".to_owned()]);
}
