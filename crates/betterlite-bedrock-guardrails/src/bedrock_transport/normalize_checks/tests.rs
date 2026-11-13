#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_translation::BedrockChecksConfigModel;
use betterlite_config::sections::guardrail_translation::bedrock_checks::{
    BedrockContentFilterCategory, BedrockContentFilterItem, BedrockContentFilterModel,
};

use crate::bedrock_transport::test_support::capture;
use crate::guardrail_checks::{ChecksInput, normalize_checks};

fn model() -> BedrockChecksConfigModel {
    BedrockChecksConfigModel {
        content_filter: Some(BedrockContentFilterModel {
            categories: vec![BedrockContentFilterItem {
                category: BedrockContentFilterCategory::Violence,
            }],
        }),
        prompt_attack: None,
        sensitive_information: None,
    }
}

#[test]
fn normalize_checks_reads_a_model_into_the_wire_keys() {
    let normalized = normalize_checks(Some(ChecksInput::Model(model())))
        .expect("normalized")
        .expect("checks");
    assert_eq!(
        normalized.keys().cloned().collect::<Vec<String>>(),
        vec!["contentFilter".to_owned()]
    );
}

#[test]
fn normalize_checks_drops_unknown_keys_and_warns() {
    let (captured, _guard) = capture();
    let input = BTreeMap::from([
        (
            "contentFilter".to_owned(),
            GuardrailValue::Object(BTreeMap::new()),
        ),
        ("unknownCheck".to_owned(), GuardrailValue::Boolean(true)),
    ]);
    let normalized = normalize_checks(Some(ChecksInput::Mapping(input)))
        .expect("normalized")
        .expect("checks");
    assert!(normalized.contains_key("contentFilter"));
    assert!(!normalized.contains_key("unknownCheck"));
    let warnings = captured.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("unknownCheck"));
}

#[test]
fn normalize_checks_drops_null_known_keys() {
    let input = BTreeMap::from([
        ("contentFilter".to_owned(), GuardrailValue::Null),
        (
            "promptAttack".to_owned(),
            GuardrailValue::Object(BTreeMap::new()),
        ),
    ]);
    let normalized = normalize_checks(Some(ChecksInput::Mapping(input)))
        .expect("normalized")
        .expect("checks");
    assert_eq!(
        normalized.keys().cloned().collect::<Vec<String>>(),
        vec!["promptAttack".to_owned()]
    );
}

#[test]
fn normalize_checks_without_checks_is_none() {
    assert!(normalize_checks(None).expect("normalized").is_none());
}

#[test]
fn normalize_checks_with_an_empty_mapping_is_none() {
    assert!(
        normalize_checks(Some(ChecksInput::Mapping(BTreeMap::new())))
            .expect("normalized")
            .is_none()
    );
}
