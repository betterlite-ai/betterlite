#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::mcp_client_side_auth::Settings;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .mcp_client_side_auth)
}

#[test]
fn header_name_defaults_to_the_legacy_header() {
    let settings = load("{}", &[]).unwrap();
    assert_eq!(settings.header_name, "x-mcp-auth");
}

#[rstest]
#[case::configured("y-mcp-auth", "y-mcp-auth")]
#[case::empty_falls_back_to_default("", "x-mcp-auth")]
fn header_name_reads_the_environment(#[case] value: &str, #[case] expected: &str) {
    let settings = load("{}", &[("LITELLM_MCP_CLIENT_SIDE_AUTH_HEADER_NAME", value)]).unwrap();
    assert_eq!(settings.header_name, expected);
}

#[rstest]
#[case::configured("y-mcp-auth", "y-mcp-auth")]
#[case::empty_falls_back_to_default("", "x-mcp-auth")]
fn header_name_reads_general_settings(#[case] value: &str, #[case] expected: &str) {
    let source = format!("general_settings:\n  mcp_client_side_auth_header_name: \"{value}\"\n");
    let settings = load(&source, &[]).unwrap();
    assert_eq!(settings.header_name, expected);
}

#[rstest]
#[case::environment_wins("env-auth", "yaml-auth", "env-auth")]
#[case::empty_environment_blocks_yaml("", "yaml-auth", "x-mcp-auth")]
fn environment_takes_precedence_over_general_settings(
    #[case] environment_value: &str,
    #[case] yaml_value: &str,
    #[case] expected: &str,
) {
    let source =
        format!("general_settings:\n  mcp_client_side_auth_header_name: \"{yaml_value}\"\n");
    let settings = load(
        &source,
        &[(
            "LITELLM_MCP_CLIENT_SIDE_AUTH_HEADER_NAME",
            environment_value,
        )],
    )
    .unwrap();
    assert_eq!(settings.header_name, expected);
}

#[test]
fn general_settings_resolves_an_environment_reference() {
    let source =
        "general_settings:\n  mcp_client_side_auth_header_name: os.environ/CUSTOM_AUTH_HEADER\n";
    let settings = load(source, &[("CUSTOM_AUTH_HEADER", "z-mcp-auth")]).unwrap();
    assert_eq!(settings.header_name, "z-mcp-auth");
}

#[test]
fn general_settings_reports_a_missing_environment_reference() {
    let source =
        "general_settings:\n  mcp_client_side_auth_header_name: os.environ/ABSENT_HEADER\n";
    let error = load(source, &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "ABSENT_HEADER" && path == "general_settings.mcp_client_side_auth_header_name"
    ));
}

#[test]
fn general_settings_rejects_a_wrong_type() {
    let error = load(
        "general_settings:\n  mcp_client_side_auth_header_name: [a]\n",
        &[],
    )
    .unwrap_err();
    assert!(matches!(error, ConfigError::Yaml { .. }));
}
