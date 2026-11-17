#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str) -> Result<super::Settings, ConfigError> {
    let lookup = BTreeMap::new();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .mcp_required_fields)
}

#[test]
fn a_missing_key_is_the_empty_list() {
    assert!(load("").unwrap().required_fields.is_empty());
    assert!(
        load("general_settings: {}")
            .unwrap()
            .required_fields
            .is_empty()
    );
    let null = load("general_settings:\n  mcp_required_fields: null").unwrap();
    assert!(null.required_fields.is_empty());
}

#[test]
fn the_list_loads_in_order() {
    let settings = load(
        "general_settings:\n  mcp_required_fields:\n    - server_name\n    - url\n    - display_name\n",
    )
    .unwrap();
    assert_eq!(
        settings.required_fields,
        ["server_name", "url", "display_name"]
    );
}

#[test]
fn an_empty_list_loads() {
    let settings = load("general_settings:\n  mcp_required_fields: []").unwrap();
    assert!(settings.required_fields.is_empty());
}

#[test]
fn a_non_list_value_is_a_config_error() {
    load("general_settings:\n  mcp_required_fields: server_name").unwrap_err();
    load("general_settings:\n  mcp_required_fields: 5").unwrap_err();
}

#[test]
fn a_non_string_entry_is_a_config_error() {
    load("general_settings:\n  mcp_required_fields:\n    - url: x\n").unwrap_err();
}
