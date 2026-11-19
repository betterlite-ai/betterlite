#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};

use super::{
    DEFAULT_MCP_PROTOCOL_VERSION, DEFAULT_SEARCH_TOOL_NAME, GATEWAY_TOKEN_VARIABLE,
    GATEWAY_URL_VARIABLE, MCP_PROTOCOL_VERSION_VARIABLE, SEARCH_TOOL_NAME_VARIABLE, Settings,
};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.bedrock_services_search)
}

fn deployment(params: &str) -> String {
    format!("model_list:\n- model_name: oracle\n  litellm_params: {params}\n")
}

fn global(params: &str) -> String {
    format!("litellm_settings: {params}\n")
}

#[derive(Clone, Copy)]
enum Key {
    GatewayUrl,
    SearchToolName,
    McpProtocolVersion,
    Unrelated,
}

#[rstest]
#[case::loads_agentcore_gateway_url(Key::GatewayUrl)]
#[case::loads_agentcore_search_tool_name(Key::SearchToolName)]
#[case::loads_agentcore_mcp_protocol_version(Key::McpProtocolVersion)]
#[case::rejects_unrelated_key(Key::Unrelated)]
fn bedrock_services_search_config_cases(#[case] key: Key) {
    match key {
        Key::GatewayUrl => assert_gateway_url(),
        Key::SearchToolName => assert_search_tool_name(),
        Key::McpProtocolVersion => assert_mcp_protocol_version(),
        Key::Unrelated => assert_unrelated_key(),
    }
}

// the deployment api_base beats the environment, the global api_base stays separate, and an
// absent or empty environment value leaves the gateway unset
fn assert_gateway_url() {
    assert_deployment_base_wins();
    assert_global_base_is_separate();
    assert_environment_base_is_optional();
    assert_missing_reference_is_reported();
}

fn assert_deployment_base_wins() {
    let settings = load(
        &deployment("{api_base: 'https://deployment.example/mcp?x=1'}"),
        &[(GATEWAY_URL_VARIABLE, "https://environment.example/mcp")],
    )
    .expect("the config should load");
    assert_eq!(
        settings.deployments[0].api_base.as_deref(),
        Some("https://deployment.example/mcp?x=1")
    );
    assert_eq!(
        settings.environment.gateway_url.as_deref(),
        Some("https://environment.example/mcp")
    );
}

fn assert_global_base_is_separate() {
    let settings = load(&global("{api_base: 'https://global.example/mcp'}"), &[])
        .expect("the config should load");
    assert_eq!(
        settings.global.api_base.as_deref(),
        Some("https://global.example/mcp")
    );
    assert_eq!(settings.deployments.len(), 0);
    let settings = load(
        &deployment("{api_base: os.environ/GATEWAY_FROM_ENV}"),
        &[("GATEWAY_FROM_ENV", "https://reference.example/mcp")],
    )
    .expect("the reference resolves");
    assert_eq!(
        settings.deployments[0].api_base.as_deref(),
        Some("https://reference.example/mcp")
    );
}

fn assert_environment_base_is_optional() {
    let settings = load("", &[]).expect("the config should load");
    assert_eq!(settings.environment.gateway_url, None);
    let settings = load("", &[(GATEWAY_URL_VARIABLE, "")]).expect("an empty value counts as unset");
    assert_eq!(settings.environment.gateway_url, None);
}

fn assert_missing_reference_is_reported() {
    let error = load(&deployment("{api_base: os.environ/MISSING_GATEWAY}"), &[])
        .expect_err("the reference cannot resolve");
    let rendered = error.to_string();
    assert!(rendered.contains("MISSING_GATEWAY"), "{rendered}");
    assert!(
        rendered.contains("model_list.0.litellm_params.api_base"),
        "{rendered}"
    );
}

// the tool name resolves per deployment, per litellm_settings and from the environment, with
// web-search-tool___WebSearch when the environment leaves it unset
fn assert_search_tool_name() {
    let source = deployment("{tool_name: custom-target___WebSearch}");
    let settings = load(&source, &[]).expect("the config should load");
    assert_eq!(
        settings.deployments[0].tool_name.as_deref(),
        Some("custom-target___WebSearch")
    );

    let settings = load(&global("{tool_name: global-target___WebSearch}"), &[])
        .expect("the config should load");
    assert_eq!(
        settings.global.tool_name.as_deref(),
        Some("global-target___WebSearch")
    );
    assert_eq!(settings.deployments.len(), 0);

    let settings = load(
        "",
        &[(SEARCH_TOOL_NAME_VARIABLE, "environment-target___WebSearch")],
    )
    .expect("the config should load");
    assert_eq!(
        settings.environment.search_tool_name,
        "environment-target___WebSearch"
    );

    let settings = load("", &[]).expect("the config should load");
    assert_eq!(
        settings.environment.search_tool_name,
        DEFAULT_SEARCH_TOOL_NAME
    );
    let settings =
        load("", &[(SEARCH_TOOL_NAME_VARIABLE, "")]).expect("an empty value counts as unset");
    assert_eq!(
        settings.environment.search_tool_name,
        DEFAULT_SEARCH_TOOL_NAME
    );
}

// the MCP revision comes from the environment and falls back to 2025-03-26
fn assert_mcp_protocol_version() {
    let settings =
        load("", &[(MCP_PROTOCOL_VERSION_VARIABLE, "2025-06-18")]).expect("the config should load");
    assert_eq!(settings.environment.mcp_protocol_version, "2025-06-18");

    let settings = load("", &[]).expect("the config should load");
    assert_eq!(
        settings.environment.mcp_protocol_version,
        DEFAULT_MCP_PROTOCOL_VERSION
    );
    let settings =
        load("", &[(MCP_PROTOCOL_VERSION_VARIABLE, "")]).expect("an empty value counts as unset");
    assert_eq!(
        settings.environment.mcp_protocol_version,
        DEFAULT_MCP_PROTOCOL_VERSION
    );
}

fn assert_unrelated_key() {
    let error =
        load(&deployment("{agentcore_unrelated_setting: 1}"), &[]).expect_err("the key is unknown");
    let ConfigError::Unsupported { paths } = error else {
        panic!("expected unsupported keys, got {error:?}");
    };
    assert_eq!(
        paths,
        ["model_list.0.litellm_params.agentcore_unrelated_setting"]
    );
}

#[derive(Clone, Copy)]
enum Credential {
    GatewayToken,
    ToolName,
}

#[rstest]
#[case::loads_agentcore_gateway_token(Credential::GatewayToken)]
#[case::loads_tool_name(Credential::ToolName)]
fn agentcore_gateway_credentials_config_cases(#[case] credential: Credential) {
    match credential {
        Credential::GatewayToken => assert_gateway_token(),
        Credential::ToolName => assert_tool_name_precedence(),
    }
}

// AGENTCORE_GATEWAY_TOKEN and a deployment api_key stay separate, an explicit api_key is never
// rewritten by the environment, and neither Debug nor an error text exposes the token
fn assert_gateway_token() {
    assert_explicit_key_is_resolved();
    assert_explicit_key_is_kept();
    assert_environment_token_is_optional();
    assert_token_is_not_debugged();
}

fn assert_explicit_key_is_resolved() {
    let source = deployment("{api_key: os.environ/GATEWAY_TOKEN_FIXTURE}");
    let settings = load(
        &source,
        &[("GATEWAY_TOKEN_FIXTURE", "fixture-explicit-token")],
    )
    .expect("the reference resolves");
    let configured = settings.deployments[0]
        .api_key
        .as_ref()
        .expect("the deployment carries the key");
    assert_eq!(configured.expose_secret(), "fixture-explicit-token");
}

fn assert_explicit_key_is_kept() {
    let settings = load(
        &deployment("{api_key: explicit-token}"),
        &[(GATEWAY_TOKEN_VARIABLE, "fixture-environment-token")],
    )
    .expect("the config should load");
    assert_eq!(
        settings.deployments[0]
            .api_key
            .as_ref()
            .expect("the deployment carries the key")
            .expose_secret(),
        "explicit-token"
    );
    assert_eq!(
        settings
            .environment
            .gateway_token
            .as_ref()
            .expect("the environment carries the token")
            .expose_secret(),
        "fixture-environment-token"
    );
}

fn assert_environment_token_is_optional() {
    let settings = load("", &[(GATEWAY_TOKEN_VARIABLE, "fixture-environment-token")])
        .expect("the config should load");
    assert_eq!(
        settings
            .environment
            .gateway_token
            .as_ref()
            .expect("the environment carries the token")
            .expose_secret(),
        "fixture-environment-token"
    );
    assert_eq!(settings.deployments.len(), 0);
    let settings = load("", &[]).expect("the config should load");
    assert_eq!(settings.environment.gateway_token, None);
    let settings =
        load("", &[(GATEWAY_TOKEN_VARIABLE, "")]).expect("an empty value counts as unset");
    assert_eq!(settings.environment.gateway_token, None);
}

fn assert_token_is_not_debugged() {
    let source = deployment("{api_key: os.environ/GATEWAY_TOKEN_FIXTURE}");
    let settings = load(
        &source,
        &[("GATEWAY_TOKEN_FIXTURE", "fixture-explicit-token")],
    )
    .expect("the reference resolves");
    let debug = format!("{settings:?}");
    assert!(!debug.contains("fixture-explicit-token"), "{debug}");
    let error = load(&deployment("{api_key: os.environ/MISSING_TOKEN}"), &[])
        .expect_err("the reference cannot resolve");
    let rendered = error.to_string();
    assert!(!rendered.contains("fixture"), "{rendered}");
}

// a deployment tool_name wins over the environment, an absent one stays None and leaves the
// environment value in place
fn assert_tool_name_precedence() {
    let settings = load(
        &deployment("{tool_name: deployment-target___WebSearch}"),
        &[(SEARCH_TOOL_NAME_VARIABLE, "environment-target___WebSearch")],
    )
    .expect("the config should load");
    assert_eq!(
        settings.deployments[0].tool_name.as_deref(),
        Some("deployment-target___WebSearch")
    );
    assert_eq!(
        settings.environment.search_tool_name,
        "environment-target___WebSearch"
    );

    let settings = load(
        &deployment("{}"),
        &[(SEARCH_TOOL_NAME_VARIABLE, "environment-target___WebSearch")],
    )
    .expect("the config should load");
    assert_eq!(settings.deployments[0].tool_name, None);
    assert_eq!(
        settings.environment.search_tool_name,
        "environment-target___WebSearch"
    );

    let settings = load(
        &global("{tool_name: global-target___WebSearch}"),
        &[(SEARCH_TOOL_NAME_VARIABLE, "environment-target___WebSearch")],
    )
    .expect("the config should load");
    assert_eq!(
        settings.global.tool_name.as_deref(),
        Some("global-target___WebSearch")
    );
    assert_eq!(settings.deployments.len(), 0);
}
