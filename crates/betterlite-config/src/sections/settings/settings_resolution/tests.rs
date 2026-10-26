#![cfg(test)]

use std::collections::BTreeMap;

use super::super::{FieldDescriptor, FieldSource, JsonValue, resolve_fields};

#[test]
fn resolution_tracks_sources_and_null_is_unset() {
    let descriptors = [
        FieldDescriptor::new("db", "DB", "DB", false, None),
        FieldDescriptor::new("env", "ENV", "ENV", false, None),
        FieldDescriptor::new("default", "DEFAULT", "DEFAULT", false, Some("fallback")),
    ];
    let database = BTreeMap::from([(String::from("DB"), JsonValue::Null)]);
    let environment = BTreeMap::from([(String::from("ENV"), String::from("value"))]);
    let (values, sources) = resolve_fields(&descriptors, &database, &environment, false);
    assert_eq!(values["db"], None);
    assert_eq!(sources["db"], FieldSource::Unset);
    assert_eq!(sources["env"], FieldSource::Env);
    assert_eq!(sources["default"], FieldSource::Default);
}
