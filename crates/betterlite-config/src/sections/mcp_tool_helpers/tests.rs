#![cfg(test)]

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};
use std::collections::BTreeMap;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .mcp_tool_helpers)
}

#[test]
fn timeout_config_cases() {
    let source = "litellm_settings:\n  mcp_client_timeout: '9'\n  mcp_tool_listing_timeout: '4'\n";
    let environment = [
        ("LITELLM_MCP_CLIENT_TIMEOUT", "12"),
        ("LITELLM_MCP_TOOL_LISTING_TIMEOUT", "7"),
    ];
    let defaults = load("", &[]).unwrap();
    assert_eq!(defaults.client_timeout.to_string(), "60");
    assert_eq!(defaults.tool_listing_timeout.to_string(), "30");
    let environment_settings = load("", &environment).unwrap();
    assert_eq!(environment_settings.client_timeout.to_string(), "12");
    assert_eq!(environment_settings.tool_listing_timeout.to_string(), "7");
    let settings = load(source, &environment).unwrap();
    assert_eq!(settings.client_timeout.to_string(), "9");
    assert_eq!(settings.tool_listing_timeout.to_string(), "4");
}

#[test]
fn invalid_timeout_is_reported() {
    assert!(matches!(
        load("", &[("LITELLM_MCP_CLIENT_TIMEOUT", "bad")]),
        Err(ConfigError::InvalidFloat { .. })
    ));
}

#[test]
fn numeric_config_timeouts_are_accepted() {
    let settings = load(
        "litellm_settings:\n  mcp_client_timeout: 9.5\n  mcp_tool_listing_timeout: 4\n",
        &[],
    )
    .unwrap();
    assert_eq!(settings.client_timeout.to_string(), "9.5");
    assert_eq!(settings.tool_listing_timeout.to_string(), "4");
}
