#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::{SectionChanges, changed_section_keys};
use crate::sections::settings::settings_rules::JsonValue;

fn section(json: &str) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(json).unwrap()
}

#[rstest]
#[case::same_rows("{\"a\": 1}", "{\"a\": 1}", "{}", &[])]
#[case::new_key("{}", "{\"a\": 1}", "{\"a\": 1}", &[])]
#[case::altered_value("{\"a\": 1, \"b\": 2}", "{\"a\": 3, \"b\": 2}", "{\"a\": 3}", &[])]
#[case::null_differs_from_missing("{}", "{\"a\": null}", "{\"a\": null}", &[])]
#[case::altered_and_removed("{\"a\": 1, \"b\": 2}", "{\"a\": 2}", "{\"a\": 2}", &["b"])]
fn reports_changed_and_removed_keys(
    #[case] baseline: &str,
    #[case] new: &str,
    #[case] changed: &str,
    #[case] removed: &[&str],
) {
    let expected = SectionChanges {
        changed: section(changed),
        removed: removed
            .iter()
            .map(|key| (*key).to_owned())
            .collect::<BTreeSet<String>>(),
    };
    assert_eq!(
        changed_section_keys(&section(baseline), &section(new)),
        expected
    );
}
