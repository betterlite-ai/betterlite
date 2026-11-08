use betterlite_db::mcp_stored_contracts::MCPTransport;
use thiserror::Error;

use super::discovery::{DiscoveryRequest, InitializeResult, build_discovery};
use super::versions::{
    GATEWAY_OPERATIONS, HANDSHAKE_VERSIONS, LATEST_HANDSHAKE_VERSION, SpecVersion,
};

pub const UNSUPPORTED_PROTOCOL_VERSION: i32 = -32022;

pub const ADVERTISED_VERSIONS_FIELD: &str = "general_settings.mcp_advertised_versions";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayVersionPolicy {
    pub versions: Vec<SpecVersion>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionRequest<'request> {
    Initialize {
        requested: &'request str,
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

impl GatewayVersionPolicy {
    pub fn negotiate<'request>(
        &self,
        request: VersionRequest<'request>,
    ) -> Result<&'request str, UnsupportedVersion> {
        let negotiated = match request {
            VersionRequest::Initialize { requested } => {
                if SpecVersion::from_wire(requested).is_some_and(SpecVersion::is_handshake) {
                    requested
                } else {
                    LATEST_HANDSHAKE_VERSION
                }
            }
            VersionRequest::Session {
                protocol_version: Some(version),
            } => version,
            VersionRequest::Session {
                protocol_version: None,
            } => return Err(self.rejection()),
        };
        if self
            .versions
            .iter()
            .any(|version| version.as_str() == negotiated)
        {
            Ok(negotiated)
        } else {
            Err(self.rejection())
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
