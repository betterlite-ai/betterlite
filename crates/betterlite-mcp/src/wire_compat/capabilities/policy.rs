use betterlite_db::mcp_stored_contracts::MCPTransport;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use thiserror::Error;

use super::discovery::{DiscoveryRequest, InitializeResult, build_discovery};
use super::versions::{
    GATEWAY_OPERATIONS, HANDSHAKE_VERSIONS, LATEST_HANDSHAKE_VERSION, SpecVersion,
};
use crate::client::sessions::negotiate::Implementation;

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

// the three fields the SDK requires of initialize params; only the version is read
#[derive(Deserialize)]
struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    #[serde(rename = "capabilities")]
    _capabilities: Object,
    #[serde(rename = "clientInfo")]
    _client_info: Implementation,
}

impl GatewayVersionPolicy {
    pub fn negotiate<'request>(
        &self,
        request: VersionRequest<'request>,
    ) -> Result<&'request str, VersionRejection> {
        let negotiated = match request {
            VersionRequest::Initialize { params } => {
                let value = serde_json::to_value(params)
                    .map_err(|error| VersionRejection::InvalidParams(error.to_string()))?;
                let parsed = serde_json::from_value::<InitializeParams>(value)
                    .map_err(|error| VersionRejection::InvalidParams(error.to_string()))?;
                SpecVersion::from_wire(&parsed.protocol_version)
                    .filter(|version| version.is_handshake())
                    .map_or(LATEST_HANDSHAKE_VERSION, SpecVersion::as_str)
            }
            VersionRequest::Session {
                protocol_version: Some(version),
            } => version,
            VersionRequest::Session {
                protocol_version: None,
            } => return Err(VersionRejection::Unsupported(self.rejection())),
        };
        if self
            .versions
            .iter()
            .any(|version| version.as_str() == negotiated)
        {
            Ok(negotiated)
        } else {
            Err(VersionRejection::Unsupported(self.rejection()))
        }
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

    fn rejection(&self) -> UnsupportedVersion {
        UnsupportedVersion {
            code: UNSUPPORTED_PROTOCOL_VERSION,
            supported: self
                .versions
                .iter()
                .map(|version| version.as_str().to_owned())
                .collect(),
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
