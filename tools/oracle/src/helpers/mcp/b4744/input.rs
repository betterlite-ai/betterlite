use std::collections::BTreeMap;

use betterlite_mcp::client::sessions::negotiate::Callback;
use betterlite_mcp::client::sessions::transport::TransportType;
use serde::Deserialize;

use super::fixture::Plan;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthKind {
    #[default]
    None,
    ApiKey,
    BearerToken,
    Basic,
    Authorization,
    Oauth2,
    Token,
    Oauth2TokenExchange,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum AuthValueInput {
    Text(String),
    Headers(BTreeMap<String, String>),
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct AuthInput {
    #[serde(default)]
    pub auth_type: AuthKind,
    #[serde(default)]
    pub auth_value: Option<AuthValueInput>,
    #[serde(default)]
    pub auth_header_name: Option<String>,
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub resolved_auth: bool,
    #[serde(default)]
    pub aws_auth: bool,
    #[serde(default)]
    pub resolved_header_name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct StdioSettings {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TlsKind {
    #[default]
    Enabled,
    Disabled,
    CaFile,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct TlsInput {
    #[serde(default)]
    pub verification: TlsKind,
    #[serde(default)]
    pub ca_file: Option<String>,
}

fn default_server_url() -> String {
    "https://mcp.example.com/mcp".to_owned()
}

const fn default_timeout_ms() -> u64 {
    60_000
}

fn default_npm_cache_dir() -> String {
    "/tmp/.npm_mcp_cache".to_owned()
}

fn default_protocol_version() -> String {
    "auto".to_owned()
}

fn default_client_version() -> String {
    "1.105.1".to_owned()
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct SettingsInput {
    #[serde(default = "default_server_url")]
    pub server_url: String,
    #[serde(default)]
    pub transport_type: TransportType,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub stdio: Option<StdioSettings>,
    #[serde(default)]
    pub auth: AuthInput,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default = "default_npm_cache_dir")]
    pub npm_cache_dir: String,
    #[serde(default)]
    pub tls: TlsInput,
    #[serde(default = "default_protocol_version")]
    pub protocol_version: String,
    #[serde(default = "default_client_version")]
    pub client_version: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TransportInput {
    #[serde(flatten)]
    pub settings: SettingsInput,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct StdioInput {
    #[serde(default)]
    pub provided: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default = "default_npm_cache_dir")]
    pub npm_cache_dir: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct InitializeInput {
    #[serde(default = "default_protocol_version")]
    pub protocol_version: String,
    #[serde(default = "default_client_version")]
    pub client_version: String,
    #[serde(default)]
    pub callbacks: Vec<Callback>,
    #[serde(default = "default_server_version")]
    pub server_version: String,
}

fn default_server_version() -> String {
    "2025-06-18".to_owned()
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct RedirectCheck {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthSourceInput {
    Resolved,
    Aws,
    Explicit,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct AuthVariant {
    #[serde(default)]
    pub sources: Vec<AuthSourceInput>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct FactoryInput {
    #[serde(flatten)]
    pub settings: SettingsInput,
    #[serde(default)]
    pub explicit_auth: bool,
    #[serde(default)]
    pub redirect_checks: Vec<RedirectCheck>,
    #[serde(default)]
    pub auth_variants: Vec<AuthVariant>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    #[default]
    Execute,
    Run,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    #[default]
    Ok,
    Protocol,
    Application,
    Cancel,
    ReadTimeout,
    RelayedTimeout,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StreamKind {
    Value,
    Http,
    Io,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Scenario {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub quiet: bool,
    #[serde(default)]
    pub plan: Plan,
    #[serde(default)]
    pub http: bool,
    #[serde(default)]
    pub operation: Operation,
    #[serde(default)]
    pub stream: Option<StreamKind>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ExecuteInput {
    pub settings: SettingsInput,
    #[serde(default)]
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Transport(TransportInput),
    StdioEnv(StdioInput),
    Initialize(InitializeInput),
    Factory(FactoryInput),
    Execute(ExecuteInput),
}
