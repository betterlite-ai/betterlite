#![cfg(test)]

use std::collections::BTreeMap;

use crate::sections::settings::{JsonValue, Section};
use super::super::SettingsStore;

fn text(value: &str) -> JsonValue {
    JsonValue::Text(value.to_owned())
}

#[test]
fn config_ownership_and_changed_rows() {
    let mut store = SettingsStore::new(Section::GeneralSettings);
    store.load_yaml(BTreeMap::from([(String::from("owned"), text("yaml"))]));
    store.apply_runtime_values(BTreeMap::from([
        (String::from("owned"), text("runtime")),
        (String::from("dynamic"), text("old")),
    ]));
    store.apply_db_row(Section::GeneralSettings, BTreeMap::from([(String::from("dynamic"), text("db"))]));
    assert!(store.set("owned", text("other")).is_err());
    assert_eq!(store.get("dynamic"), Some(&text("db")));
    assert_eq!(store.get("owned"), Some(&text("yaml")));
}
