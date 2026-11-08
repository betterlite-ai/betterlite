use betterlite_db::mcp_stored_contracts::MCPTransport;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde::de::{Error as _, value::Error as TokenError};
use std::collections::HashMap;
use thiserror::Error;

use super::discovery::{DiscoveryRequest, InitializeResult, build_discovery};
use super::versions::{
    GATEWAY_OPERATIONS, HANDSHAKE_VERSIONS, LATEST_HANDSHAKE_VERSION, SpecVersion,
};
use crate::wire_compat::result_conversion::Icon;

pub const UNSUPPORTED_PROTOCOL_VERSION: i32 = -32022;
pub const INVALID_PARAMS: i32 = -32602;
pub const ADVERTISED_VERSIONS_FIELD: &str = "general_settings.mcp_advertised_versions";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayVersionPolicy {
    pub versions: Vec<SpecVersion>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionRequest<'request> {
    Initialize {
        params: &'request Object,
    },
    Session {
        protocol_version: Option<&'request str>,
    },
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Unsupported MCP protocol version")]
pub struct UnsupportedVersion {
    pub code: i32,
    pub supported: Vec<String>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum VersionRejection {
    #[error(transparent)]
    Unsupported(UnsupportedVersion),
    #[error("invalid initialize params: {0}")]
    InvalidParams(String),
}

impl VersionRejection {
    #[must_use]
    pub const fn code(&self) -> i32 {
        match self {
            Self::Unsupported(rejection) => rejection.code,
            Self::InvalidParams(_) => INVALID_PARAMS,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
    #[serde(rename = "_meta")]
    pub meta: Option<RequestMeta>,
}

#[derive(Deserialize)]
pub struct ClientCapabilities {
    pub experimental: Option<HashMap<String, Object>>,
    pub extensions: Option<HashMap<String, Object>>,
    pub sampling: Option<SamplingCapability>,
    pub elicitation: Option<ElicitationCapability>,
    pub roots: Option<RootsCapability>,
    pub tasks: Option<TasksCapability>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestMeta {
    #[serde(default)]
    pub progress_token: ProgressToken,
}

// an Option reads null as absent; pydantic's str | int also takes a bool and a whole number of any size
#[derive(Default, Deserialize)]
#[serde(try_from = "Json")]
pub struct ProgressToken;

impl TryFrom<Json> for ProgressToken {
    type Error = TokenError;
    fn try_from(token: Json) -> Result<Self, Self::Error> {
        match token {
            Json::Text(_) | Json::Bool(_) => Ok(Self),
            Json::Number(number)
                if !number.to_string().contains(['.', 'e', 'E'])
                    || number
                        .as_f64()
                        .is_some_and(|float| float.fract().abs() < f64::EPSILON) =>
            {
                Ok(Self)
            }
            Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_) => Err(
                TokenError::custom("progressToken must be a string or an integer"),
            ),
        }
    }
}

#[derive(Deserialize)]
pub struct SamplingCapability {
    pub context: Option<Object>,
    pub tools: Option<Object>,
}

#[derive(Deserialize)]
pub struct ElicitationCapability {
    pub form: Option<Object>,
    pub url: Option<Object>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootsCapability {
    pub list_changed: Option<bool>,
}

#[derive(Deserialize)]
pub struct TasksCapability {
    pub list: Option<Object>,
    pub cancel: Option<Object>,
    pub requests: Option<TaskRequests>,
}

#[derive(Deserialize)]
pub struct TaskRequests {
    pub sampling: Option<TaskSampling>,
    pub elicitation: Option<TaskElicitation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSampling {
    pub create_message: Option<Object>,
}

#[derive(Deserialize)]
pub struct TaskElicitation {
    pub create: Option<Object>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub website_url: Option<String>,
    pub icons: Option<Vec<Icon>>,
}

impl GatewayVersionPolicy {
    pub fn negotiate<'request>(
        &self,
        request: VersionRequest<'request>,
    ) -> Result<&'request str, VersionRejection> {
        let negotiated = match request {
            VersionRequest::Initialize { params } => {
                let parsed = serde_json::to_value(params)
                    .and_then(serde_json::from_value::<InitializeParams>)
                    .map_err(|error| VersionRejection::InvalidParams(error.to_string()))?;
                SpecVersion::from_wire(&parsed.protocol_version)
                    .filter(|version| version.is_handshake())
                    .map_or(LATEST_HANDSHAKE_VERSION, SpecVersion::as_str)
            }
            VersionRequest::Session { protocol_version } => protocol_version.unwrap_or_default(),
        };
        self.versions
            .iter()
            .any(|version| version.as_str() == negotiated)
            .then_some(negotiated)
            .ok_or_else(|| {
                VersionRejection::Unsupported(UnsupportedVersion {
                    code: UNSUPPORTED_PROTOCOL_VERSION,
                    supported: self
                        .versions
                        .iter()
                        .map(|version| version.as_str().to_owned())
                        .collect(),
                })
            })
    }

    #[must_use]
    pub fn apply_initialize(&self, initialized: InitializeResult) -> InitializeResult {
        let discovery = build_discovery(DiscoveryRequest {
            configured: &self.versions,
            revision: SpecVersion::from_wire(&initialized.protocol_version),
            transport: MCPTransport::Http,
            authorized_operations: &GATEWAY_OPERATIONS,
            upstream_versions: &HANDSHAKE_VERSIONS,
            capabilities: initialized.capabilities.clone(),
            client_extensions: &[],
            upstream_extensions: &[],
            instructions: initialized.instructions.as_deref(),
        });
        InitializeResult {
            protocol_version: initialized.protocol_version,
            capabilities: discovery.capabilities,
            server_info: initialized.server_info,
            instructions: initialized.instructions,
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ConfigError {
    #[error("{field} must name at least one MCP protocol version")]
    EmptyAdvertisedVersions { field: &'static str },
    #[error("{field} names {value}, which is not a legacy MCP protocol version")]
    UnknownAdvertisedVersion { field: &'static str, value: String },
}

pub fn advertised_versions(configured: Option<&[String]>) -> Result<Vec<SpecVersion>, ConfigError> {
    match configured {
        None => Ok(HANDSHAKE_VERSIONS.to_vec()),
        Some([]) => Err(ConfigError::EmptyAdvertisedVersions {
            field: ADVERTISED_VERSIONS_FIELD,
        }),
        Some(values) => values
            .iter()
            .map(|value| match SpecVersion::from_wire(value) {
                Some(version) if version.is_handshake() => Ok(version),
                Some(_) | None => Err(ConfigError::UnknownAdvertisedVersion {
                    field: ADVERTISED_VERSIONS_FIELD,
                    value: value.clone(),
                }),
            })
            .collect(),
    }
}

mod tests;
