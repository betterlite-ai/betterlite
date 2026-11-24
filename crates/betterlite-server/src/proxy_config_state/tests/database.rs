use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::settings::{JsonValue, Section};

use super::support::{self, json};
use crate::proxy_config_state::database::merge_section_values;
use crate::proxy_config_state::database::postgres;
use crate::proxy_config_state::error::ConfigStateError;

fn keys(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn the_section_merge_drops_removed_keys_and_applies_changed_ones() {
    let existing = support::values(&[("kept", json!(1)), ("gone", json!(2))]);
    let changed = support::values(&[("new", json!(3)), ("kept", json!(4))]);
    let merged = merge_section_values(&existing, &changed, &keys(&["gone"])).unwrap();
    assert_eq!(
        merged,
        support::values(&[("kept", json!(4)), ("new", json!(3))])
    );
}

#[test]
fn the_section_merge_skips_a_row_that_already_matches() {
    let existing = support::values(&[("a", json!(2))]);
    let changed = support::values(&[("a", json!(2))]);
    assert_eq!(
        merge_section_values(&existing, &changed, &BTreeSet::new()),
        None
    );
}

#[test]
fn a_stored_section_decodes_json_text_and_an_object() {
    let object = json!({"a": 1, "b": [true, "text"]});
    let expected = support::values(&[("a", json!(1)), ("b", json!([true, "text"]))]);
    let stored = postgres::encode_config_value(&object).unwrap();
    assert!(stored.starts_with('"'));
    assert_eq!(
        postgres::decode_section_values(Section::GeneralSettings, &stored).unwrap(),
        expected
    );
    assert_eq!(
        postgres::decode_section_values(Section::GeneralSettings, &object.to_string()).unwrap(),
        expected
    );
}

#[test]
fn a_stored_section_that_is_not_an_object_is_rejected() {
    let error = postgres::decode_section_values(Section::GeneralSettings, "[1, 2]").unwrap_err();
    let ConfigStateError::InvalidStoredSection { section } = error else {
        panic!("expected an invalid stored section");
    };
    assert_eq!(section, "general_settings");
}

#[test]
fn a_stored_null_section_reads_as_an_empty_row() {
    let empty = BTreeMap::<String, JsonValue>::new();
    assert_eq!(
        postgres::decode_section_values(Section::GeneralSettings, "null").unwrap(),
        empty
    );
}

#[test]
fn the_section_merge_holds_litellms_lock_statement() {
    assert_eq!(
        postgres::CONFIG_SECTION_LOCK_SQL,
        "SELECT 1 AS locked FROM pg_advisory_xact_lock(hashtext($1))"
    );
}
