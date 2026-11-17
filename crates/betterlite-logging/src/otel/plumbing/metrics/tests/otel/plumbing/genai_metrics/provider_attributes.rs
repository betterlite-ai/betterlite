use std::collections::BTreeMap;

use betterlite_logging::otel::legacy_metrics::attributes::CommonAttributes;
use betterlite_logging::otel::plumbing::metrics::attributes::provider_attributes;
use rstest::rstest;

use super::support::{DURATION, attributes_of, record};

fn labeled(attributes: CommonAttributes) -> BTreeMap<String, String> {
    attributes
        .into_iter()
        .map(|(key, value)| (key, value.expect("provider labels always carry a value")))
        .collect()
}

#[test]
fn a_mapped_provider_sets_the_semconv_name_and_keeps_the_raw_system_label() {
    let expected = BTreeMap::from([
        (
            "gen_ai.provider.name".to_owned(),
            "azure.ai.openai".to_owned(),
        ),
        ("gen_ai.system".to_owned(), "azure".to_owned()),
    ]);
    assert_eq!(labeled(provider_attributes(Some("azure"))), expected);
}

#[test]
fn an_unmapped_provider_keeps_its_own_name_in_both_labels() {
    let expected = BTreeMap::from([
        ("gen_ai.provider.name".to_owned(), "custom".to_owned()),
        ("gen_ai.system".to_owned(), "custom".to_owned()),
    ]);
    assert_eq!(labeled(provider_attributes(Some("custom"))), expected);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn a_call_without_a_provider_gets_no_provider_labels(#[case] provider: Option<&str>) {
    assert!(provider_attributes(provider).is_empty());
}

#[test]
fn the_recorder_labels_a_call_with_the_provider_from_litellm_params() {
    let recorded = record("{}", None);
    let attributes = attributes_of(&recorded, DURATION);
    assert_eq!(attributes["gen_ai.provider.name"], "openai");
    assert_eq!(attributes["gen_ai.system"], "openai");
}

#[rstest]
#[case::missing(r#"{"litellm_params": {}}"#)]
#[case::not_text(r#"{"litellm_params": {"custom_llm_provider": 3}}"#)]
fn a_call_whose_provider_is_not_a_name_has_neither_label(#[case] extra: &str) {
    let recorded = record(extra, None);
    let attributes = attributes_of(&recorded, DURATION);
    assert!(!attributes.contains_key("gen_ai.provider.name"));
    assert!(!attributes.contains_key("gen_ai.system"));
}
