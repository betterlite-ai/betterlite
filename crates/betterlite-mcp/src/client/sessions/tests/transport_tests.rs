use std::collections::BTreeMap;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::client::sessions::environment::safe_stdio_env;
use crate::client::sessions::error::{SessionConfigError, SessionError};
use crate::client::sessions::factory::AuthSource;
use crate::client::sessions::transport::{StdioConfig, TransportType};
use crate::credentials::{AuthValue, McpAuthType};
use secrecy::SecretString;

use super::fixture::{FakeRuntime, Plan, build_client, make_config};

fn stdio_config(env: Option<BTreeMap<String, String>>) -> StdioConfig {
    StdioConfig {
        command: "npx".to_owned(),
        args: vec!["-y".to_owned(), "@modelcontextprotocol/server".to_owned()],
        env,
    }
}

fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("PATH".to_owned(), "/usr/bin".to_owned()),
        ("HOME".to_owned(), "/home/user".to_owned()),
        ("OPENAI_API_KEY".to_owned(), "sk-secret".to_owned()),
    ])
}

#[tokio::test]
async fn stdio_transport_passes_command_args_and_the_safe_environment() {
    let runtime = FakeRuntime::new(Plan::default());
    let client = build_client(
        runtime,
        make_config(|value| {
            value.transport_type = TransportType::Stdio;
            value.stdio = Some(stdio_config(None));
            value.environment = environment();
            value.npm_cache_dir = "/cache".to_owned();
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = client.create_transport_context().unwrap();
    assert!(contexts.http_client.is_none());
    let parameters = client.runtime.stdio().unwrap();
    assert_eq!(parameters.command, "npx");
    assert_eq!(
        parameters.args,
        vec!["-y".to_owned(), "@modelcontextprotocol/server".to_owned()]
    );
    assert_eq!(
        parameters.env.get("PATH").map(String::as_str),
        Some("/usr/bin")
    );
    assert_eq!(
        parameters.env.get("NPM_CONFIG_CACHE").map(String::as_str),
        Some("/cache")
    );
    assert!(!parameters.env.contains_key("OPENAI_API_KEY"));
}

#[tokio::test]
async fn stdio_transport_requires_a_configuration() {
    let runtime = FakeRuntime::new(Plan::default());
    let client = build_client(
        runtime,
        make_config(|value| value.transport_type = TransportType::Stdio),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let error = client.create_transport_context().err().unwrap();
    assert!(matches!(
        error,
        SessionError::Configuration(SessionConfigError::MissingStdioConfig)
    ));
}

#[tokio::test]
async fn a_provided_stdio_environment_is_kept_and_gets_the_sdk_defaults_under_it() {
    let provided = BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-secret".to_owned())]);
    let runtime = FakeRuntime::new(Plan::default());
    let client = build_client(
        runtime,
        make_config(|value| {
            value.transport_type = TransportType::Stdio;
            value.stdio = Some(stdio_config(Some(provided.clone())));
            value.environment = environment();
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    client.create_transport_context().unwrap();
    let parameters = client.runtime.stdio().unwrap();
    assert_eq!(parameters.env, provided);
    assert_eq!(
        parameters.defaults.get("PATH").map(String::as_str),
        Some("/usr/bin")
    );
    assert_eq!(
        parameters.defaults.get("HOME").map(String::as_str),
        Some("/home/user")
    );
    assert!(!parameters.defaults.contains_key("OPENAI_API_KEY"));
}

#[tokio::test]
async fn the_sdk_defaults_skip_shell_function_values() {
    let runtime = FakeRuntime::new(Plan::default());
    let client = build_client(
        runtime,
        make_config(|value| {
            value.transport_type = TransportType::Stdio;
            value.stdio = Some(stdio_config(Some(BTreeMap::new())));
            value.environment = BTreeMap::from([
                ("PATH".to_owned(), "/usr/bin".to_owned()),
                ("TERM".to_owned(), "() { :; }; cmd".to_owned()),
            ]);
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    client.create_transport_context().unwrap();
    let parameters = client.runtime.stdio().unwrap();
    assert_eq!(
        parameters.defaults.get("PATH").map(String::as_str),
        Some("/usr/bin")
    );
    assert!(!parameters.defaults.contains_key("TERM"));
}

#[test]
fn safe_environment_keeps_only_allowlisted_variables() {
    let safe = safe_stdio_env(None, &environment(), "/cache");
    assert_eq!(safe.get("PATH").map(String::as_str), Some("/usr/bin"));
    assert_eq!(safe.get("HOME").map(String::as_str), Some("/home/user"));
    assert!(!safe.contains_key("OPENAI_API_KEY"));
    assert_eq!(safe.len(), 3);
}

#[test]
fn safe_environment_injects_the_npm_cache_directory() {
    let safe = safe_stdio_env(None, &environment(), "/cache");
    assert_eq!(
        safe.get("NPM_CONFIG_CACHE").map(String::as_str),
        Some("/cache")
    );
}

#[test]
fn safe_environment_keeps_an_existing_npm_cache() {
    let mut environment = environment();
    environment.insert("NPM_CONFIG_CACHE".to_owned(), "/existing".to_owned());
    let safe = safe_stdio_env(None, &environment, "/cache");
    assert_eq!(
        safe.get("NPM_CONFIG_CACHE").map(String::as_str),
        Some("/existing")
    );
}

#[tokio::test]
async fn sse_transport_carries_url_timeout_headers_and_a_client_config() {
    let runtime = FakeRuntime::new(Plan::default());
    let client = build_client(
        runtime,
        make_config(|value| {
            value.transport_type = TransportType::Sse;
            value.timeout = Duration::from_millis(1500);
            value.auth_type = McpAuthType::BearerToken;
            value.auth_value = Some(AuthValue::Text(SecretString::from("token".to_owned())));
            value.extra_headers = BTreeMap::from([("X-Extra".to_owned(), "yes".to_owned())]);
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = client.create_transport_context().unwrap();
    assert!(contexts.http_client.is_none());
    let request = client.runtime.sse().unwrap();
    assert_eq!(request.url, "https://mcp.example.com/mcp");
    assert_eq!(request.timeout, Duration::from_millis(1500));
    assert_eq!(
        request.headers.get("Authorization").map(String::as_str),
        Some("Bearer token")
    );
    assert_eq!(
        request.headers.get("X-Extra").map(String::as_str),
        Some("yes")
    );
    assert_eq!(request.client.headers, request.headers);
    assert_eq!(request.client.timeout, Duration::from_millis(1500));
    assert_eq!(request.client.auth, AuthSource::None);
    assert!(request.client.follow_redirects);
    assert!(request.client.capture_error_responses);
    let rendered = format!("{request:?}");
    assert!(rendered.contains("Authorization"));
    assert!(!rendered.contains("token"));
}

#[tokio::test]
async fn http_transport_builds_a_client_and_hands_it_to_the_streaming_transport() {
    let runtime = FakeRuntime::new(Plan::default());
    let client = build_client(
        runtime,
        make_config(|value| {
            value.transport_type = TransportType::Http;
            value.timeout = Duration::from_millis(2500);
            value.auth_type = McpAuthType::ApiKey;
            value.auth_value = Some(AuthValue::Text(SecretString::from("key".to_owned())));
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = client.create_transport_context().unwrap();
    assert!(contexts.http_client.is_some());
    let configs = client.runtime.http_configs();
    assert_eq!(configs.len(), 1);
    assert_eq!(
        configs[0].headers.get("X-API-Key").map(String::as_str),
        Some("key")
    );
    assert_eq!(configs[0].timeout, Duration::from_millis(2500));
    assert_eq!(
        client.runtime.urls(),
        vec!["https://mcp.example.com/mcp".to_owned()]
    );
}
