#![cfg(test)]

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};
use rstest::rstest;
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

#[rstest]
#[case::defaults("", &[], ("60", "30"))]
#[case::environment("", &[("LITELLM_MCP_CLIENT_TIMEOUT", "12"), ("LITELLM_MCP_TOOL_LISTING_TIMEOUT", "7")], ("12", "7"))]
#[case::configured("litellm_settings:\n  mcp_client_timeout: '9'\n  mcp_tool_listing_timeout: '4'\n", &[("LITELLM_MCP_CLIENT_TIMEOUT", "12"), ("LITELLM_MCP_TOOL_LISTING_TIMEOUT", "7")], ("9", "4"))]
#[case::reference("litellm_settings:\n  mcp_client_timeout: os.environ/CUSTOM\n", &[("CUSTOM", "2.5")], ("2.5", "30"))]
fn timeout_config_cases(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: (&str, &str),
) {
    let settings = load(source, environment).unwrap();
    assert_eq!(settings.client_timeout.to_string(), expected.0);
    assert_eq!(settings.tool_listing_timeout.to_string(), expected.1);
}

#[test]
fn settings_defaults_match_loaded_defaults() {
    let defaults = Settings::default();
    let loaded = load("", &[]).unwrap();
    assert_eq!(defaults, loaded);
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
