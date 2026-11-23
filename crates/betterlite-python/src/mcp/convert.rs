use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::tls::VerifySetting;
use betterlite_http::mcp_sigv4::McpSigV4Settings;
use betterlite_mcp::client::sessions::negotiate::ProtocolVersion;
use betterlite_mcp::client::sessions::transport::{StdioConfig, TransportType};
use betterlite_mcp::credentials::{AuthValue, McpAuthType};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

mod payload;
mod request;
mod tests;

pub(crate) use payload::{NativeMcpErrorPayload, session_payload, tool_failure};
pub(crate) use request::{
    CallToolWire, GetPromptWire, call_request, deadline, prompt_params, tool_format,
};

// litellm/constants.py reads these at import; an empty variable counts as unset (docs/compatibility.md)
const CLIENT_TIMEOUT_VARIABLE: &str = "LITELLM_MCP_CLIENT_TIMEOUT";
const CLIENT_TIMEOUT_SECONDS: f64 = 60.0;
const LISTING_TIMEOUT_VARIABLE: &str = "LITELLM_MCP_TOOL_LISTING_TIMEOUT";
const LISTING_TIMEOUT_SECONDS: f64 = 30.0;
const NPM_CACHE_VARIABLE: &str = "MCP_NPM_CACHE_DIR";
const NPM_CACHE_DIR: &str = "/tmp/.npm_mcp_cache";
const AUTO_PROTOCOL_VERSION: &str = "auto";

// The settings text B8082 builds from LiteLLM's MCPClient constructor arguments (client.py#L376-L430)
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ClientWire {
    pub(crate) server_url: Option<String>,
    pub(crate) transport_type: Option<TransportType>,
    pub(crate) auth_type: Option<AuthTypeWire>,
    pub(crate) auth_value: Option<AuthValueWire>,
    pub(crate) auth_header_name: Option<String>,
    pub(crate) timeout: Option<f64>,
    pub(crate) stdio_config: Option<StdioWire>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) ssl_verify: Option<SslVerifyWire>,
    pub(crate) protocol_version: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthTypeWire {
    None,
    ApiKey,
    BearerToken,
    Basic,
    Authorization,
    Oauth2,
    AwsSigv4,
    Token,
    Oauth2TokenExchange,
    Oauth2IdJag,
    TruePassthrough,
    OauthDelegate,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum AuthValueWire {
    Text(String),
    Headers(BTreeMap<String, String>),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum SslVerifyWire {
    Enabled(bool),
    Path(String),
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct StdioWire {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: Option<BTreeMap<String, String>>,
}

// Everything the settings text decides, with the process environment and every default already applied
pub(crate) struct ResolvedSettings {
    pub(crate) server_url: String,
    pub(crate) transport_type: TransportType,
    pub(crate) auth_type: McpAuthType,
    pub(crate) auth_value: Option<AuthValue>,
    pub(crate) auth_header_name: Option<String>,
    pub(crate) extra_headers: BTreeMap<String, String>,
    pub(crate) timeout: Duration,
    pub(crate) listing_timeout: Duration,
    pub(crate) stdio: Option<StdioConfig>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) npm_cache_dir: String,
    pub(crate) ssl_verify: Option<VerifySetting>,
    pub(crate) protocol_version: ProtocolVersion,
    pub(crate) aws: Option<McpSigV4Settings>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the MCP text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the MCP answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    serde_json::from_str(text).map_err(|source| ConvertError::Json { source })
}

pub(crate) fn encode<T>(value: &T) -> Result<String, ConvertError>
where
    T: Serialize,
{
    serde_json::to_string(value).map_err(|source| ConvertError::Encode { source })
}

pub(crate) fn resolve_settings(
    wire: ClientWire,
    environment: &BTreeMap<String, String>,
) -> Result<ResolvedSettings, NativeMcpErrorPayload> {
    let kind = wire.auth_type.unwrap_or(AuthTypeWire::None);
    let stored = wire.auth_value.map(auth_value);
    let aws = if kind == AuthTypeWire::AwsSigv4 {
        Some(aws_settings(stored.as_ref())?)
    } else {
        None
    };
    let timeout = match wire.timeout {
        Some(seconds) => seconds_duration("timeout", seconds)?,
        None => environment_duration(CLIENT_TIMEOUT_VARIABLE, environment, CLIENT_TIMEOUT_SECONDS)?,
    };
    let protocol = wire
        .protocol_version
        .unwrap_or_else(|| AUTO_PROTOCOL_VERSION.to_owned());
    Ok(ResolvedSettings {
        server_url: wire.server_url.unwrap_or_default(),
        transport_type: wire.transport_type.unwrap_or_default(),
        auth_type: auth_kind(kind)?,
        auth_value: if aws.is_some() { None } else { stored },
        auth_header_name: wire.auth_header_name,
        extra_headers: wire.extra_headers.unwrap_or_default(),
        timeout,
        listing_timeout: environment_duration(
            LISTING_TIMEOUT_VARIABLE,
            environment,
            LISTING_TIMEOUT_SECONDS,
        )?,
        stdio: wire.stdio_config.map(|stdio| StdioConfig {
            command: stdio.command,
            args: stdio.args,
            env: stdio.env,
        }),
        environment: environment.clone(),
        npm_cache_dir: environment
            .get(NPM_CACHE_VARIABLE)
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| NPM_CACHE_DIR.to_owned()),
        ssl_verify: wire.ssl_verify.map(|verify| match verify {
            SslVerifyWire::Enabled(enabled) => VerifySetting::Bool(enabled),
            SslVerifyWire::Path(path) => VerifySetting::Path(path),
        }),
        protocol_version: ProtocolVersion::try_from(protocol)
            .map_err(|error| NativeMcpErrorPayload::value(error.to_string()))?,
        aws,
    })
}

pub(crate) fn auth_value(wire: AuthValueWire) -> AuthValue {
    match wire {
        AuthValueWire::Text(text) => AuthValue::Text(SecretString::from(text)),
        AuthValueWire::Headers(headers) => {
            let secrets = headers
                .into_iter()
                .map(|(name, value)| (name, SecretString::from(value)));
            AuthValue::Headers(secrets.collect())
        }
    }
}

// LiteLLM picks aws_sigv4 signing by auth_type and reads its keys from the auth_value map (B4742)
fn aws_settings(value: Option<&AuthValue>) -> Result<McpSigV4Settings, NativeMcpErrorPayload> {
    let headers = match value {
        Some(AuthValue::Headers(headers)) => headers,
        Some(AuthValue::Text(_)) => {
            return Err(NativeMcpErrorPayload::value(
                "auth_type aws_sigv4 needs auth_value to be a map of AWS settings".to_owned(),
            ));
        }
        None => return Ok(McpSigV4Settings::default()),
    };
    let text = |name: &str| {
        headers
            .get(name)
            .map(|secret| secret.expose_secret().to_owned())
    };
    Ok(McpSigV4Settings {
        aws_access_key_id: text("aws_access_key_id").map(SecretString::from),
        aws_secret_access_key: text("aws_secret_access_key").map(SecretString::from),
        aws_session_token: text("aws_session_token").map(SecretString::from),
        aws_region_name: text("aws_region_name"),
        aws_service_name: text("aws_service_name"),
        aws_role_name: text("aws_role_name"),
        aws_session_name: text("aws_session_name"),
    })
}

fn auth_kind(kind: AuthTypeWire) -> Result<McpAuthType, NativeMcpErrorPayload> {
    match kind {
        AuthTypeWire::None | AuthTypeWire::AwsSigv4 => Ok(McpAuthType::None),
        AuthTypeWire::ApiKey => Ok(McpAuthType::ApiKey),
        AuthTypeWire::BearerToken => Ok(McpAuthType::BearerToken),
        AuthTypeWire::Basic => Ok(McpAuthType::Basic),
        AuthTypeWire::Authorization => Ok(McpAuthType::Authorization),
        AuthTypeWire::Oauth2 => Ok(McpAuthType::Oauth2),
        AuthTypeWire::Token => Ok(McpAuthType::Token),
        AuthTypeWire::Oauth2TokenExchange => Ok(McpAuthType::Oauth2TokenExchange),
        AuthTypeWire::Oauth2IdJag => Err(unsupported_auth("oauth2_id_jag")),
        AuthTypeWire::TruePassthrough => Err(unsupported_auth("true_passthrough")),
        AuthTypeWire::OauthDelegate => Err(unsupported_auth("oauth_delegate")),
    }
}

fn unsupported_auth(name: &str) -> NativeMcpErrorPayload {
    NativeMcpErrorPayload::value(format!(
        "auth_type {name} is not supported by the native MCP client"
    ))
}

fn seconds_duration(field: &str, seconds: f64) -> Result<Duration, NativeMcpErrorPayload> {
    Duration::try_from_secs_f64(seconds).map_err(|error| {
        NativeMcpErrorPayload::value(format!("{field} is not a valid number of seconds: {error}"))
    })
}

fn environment_duration(
    variable: &str,
    environment: &BTreeMap<String, String>,
    fallback: f64,
) -> Result<Duration, NativeMcpErrorPayload> {
    let text = environment.get(variable).map_or("", String::as_str);
    let seconds = betterlite_env::number::<f64>(variable, text)
        .map_err(|error| NativeMcpErrorPayload::value(error.to_string()))?
        .unwrap_or(fallback);
    seconds_duration(variable, seconds)
}
