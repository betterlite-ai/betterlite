#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::custom_labels_from_metadata;
use crate::prometheus_label_projection::test_support::texts;

const METADATA: &str = r#"{
    "region": "eu",
    "count": 2,
    "enabled": true,
    "absent": null,
    "list": ["a"],
    "nested": {"region": "us", "scalar": "flat", "deeper": {"zone": "z1"}}
}"#;

fn metadata() -> JsonObject {
    serde_json::from_str(METADATA).expect("the fixture is an object")
}

#[rstest]
#[case::plain_key(&["region"], &[("region", "eu")])]
#[case::one_metadata_prefix(&["metadata.region"], &[("metadata_region", "eu")])]
#[case::nested_key(&["nested.region"], &[("nested_region", "us")])]
#[case::nested_after_a_prefix(&["metadata.nested.deeper.zone"], &[("metadata_nested_deeper_zone", "z1")])]
#[case::missing_key(&["missing"], &[])]
#[case::traversal_through_a_string(&["nested.region.deeper"], &[])]
#[case::number_value(&["count"], &[])]
#[case::boolean_value(&["enabled"], &[])]
#[case::null_value(&["absent"], &[])]
#[case::list_value(&["list"], &[])]
#[case::object_value(&["nested"], &[])]
#[case::several_keys(&["region", "nested.region"], &[("region", "eu"), ("nested_region", "us")])]
#[case::no_configured_keys(&[], &[])]
fn a_configured_key_names_a_string_label(
    #[case] configured: &[&str],
    #[case] expected: &[(&str, &str)],
) {
    let labels = custom_labels_from_metadata(&metadata(), &texts(configured));
    let expected: BTreeMap<String, String> = expected
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(labels, expected);
}
