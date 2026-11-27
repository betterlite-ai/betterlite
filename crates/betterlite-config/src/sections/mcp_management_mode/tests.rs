#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::ManagementMode;
use crate::loader::{ConfigError, ConfigLoader};

fn loaded_mode(source: &str) -> Result<ManagementMode, ConfigError> {
    Ok(ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)?
    .sections
    .mcp_management_mode
    .mode)
}

fn loaded_mode_with(source: &str, variable: &str, value: &str) -> ManagementMode {
    let lookup = BTreeMap::from([(variable.to_owned(), value.to_owned())]);
    ConfigLoader { lookup }
        .load(source)
        .unwrap()
        .sections
        .mcp_management_mode
        .mode
}

#[rstest]
#[case::absent("", ManagementMode::Restricted)]
#[case::null(
    "general_settings:\n  user_mcp_management_mode: null",
    ManagementMode::Restricted
)]
#[case::restricted(
    "general_settings:\n  user_mcp_management_mode: restricted",
    ManagementMode::Restricted
)]
#[case::view_all(
    "general_settings:\n  user_mcp_management_mode: view_all",
    ManagementMode::ViewAll
)]
#[case::upper_case(
    "general_settings:\n  user_mcp_management_mode: VIEW_ALL",
    ManagementMode::Restricted
)]
#[case::empty(
    "general_settings:\n  user_mcp_management_mode: \"\"",
    ManagementMode::Restricted
)]
fn restricted_unless_exactly_view_all(#[case] source: &str, #[case] expected: ManagementMode) {
    assert_eq!(loaded_mode(source).unwrap(), expected);
}

#[test]
fn an_environment_reference_resolves_before_the_comparison() {
    let source = "general_settings:\n  user_mcp_management_mode: os.environ/MCP_MANAGEMENT_MODE";
    assert_eq!(
        loaded_mode_with(source, "MCP_MANAGEMENT_MODE", "view_all"),
        ManagementMode::ViewAll
    );
    assert_eq!(
        loaded_mode_with(source, "MCP_MANAGEMENT_MODE", "restricted"),
        ManagementMode::Restricted
    );
}

#[test]
fn an_empty_environment_value_is_restricted() {
    let source = "general_settings:\n  user_mcp_management_mode: os.environ/MCP_MANAGEMENT_MODE";
    assert_eq!(
        loaded_mode_with(source, "MCP_MANAGEMENT_MODE", ""),
        ManagementMode::Restricted
    );
}

#[test]
fn a_non_string_value_is_a_load_error() {
    loaded_mode("general_settings:\n  user_mcp_management_mode: true").unwrap_err();
    loaded_mode("general_settings:\n  user_mcp_management_mode: 5").unwrap_err();
}
