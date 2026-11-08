use betterlite_db::mcp_stored_contracts::MCPTransport;
use betterlite_mcp::wire_compat::capabilities::discovery::{
    DiscoveryRequest, InitializeResult, ServerCapabilities, build_discovery,
};
use betterlite_mcp::wire_compat::capabilities::policy::{
    GatewayVersionPolicy, VersionRejection, VersionRequest, advertised_versions,
};
use betterlite_mcp::wire_compat::capabilities::versions::SpecVersion;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperResult;

use super::failure;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscoveryFields {
    configured: Vec<String>,
    revision: String,
    transport: MCPTransport,
    authorized_operations: Vec<String>,
    upstream_versions: Vec<String>,
    capabilities: ServerCapabilities,
    #[serde(default)]
    client_extensions: Vec<String>,
    #[serde(default)]
    upstream_extensions: Vec<String>,
    #[serde(default)]
    instructions: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolicyFields {
    versions: Vec<String>,
    method: String,
    #[serde(default)]
    params: Option<Object>,
    #[serde(default)]
    protocol_version: Option<String>,
    initialized: Object,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdvertisedFields {
    #[serde(default)]
    configured: Option<Vec<String>>,
}

fn known_versions(wire: &[String]) -> Vec<SpecVersion> {
    wire.iter()
        .filter_map(|version| SpecVersion::from_wire(version))
        .collect()
}

pub(crate) fn run_discovery(fields: &DiscoveryFields) -> Value {
    let operations: Vec<&str> = fields
        .authorized_operations
        .iter()
        .map(String::as_str)
        .collect();
    let client_extensions: Vec<&str> = fields
        .client_extensions
        .iter()
        .map(String::as_str)
        .collect();
    let upstream_extensions: Vec<&str> = fields
        .upstream_extensions
        .iter()
        .map(String::as_str)
        .collect();
    let discovery = build_discovery(DiscoveryRequest {
        configured: &known_versions(&fields.configured),
        revision: SpecVersion::from_wire(&fields.revision),
        transport: fields.transport,
        authorized_operations: &operations,
        upstream_versions: &known_versions(&fields.upstream_versions),
        capabilities: fields.capabilities.clone(),
        client_extensions: &client_extensions,
        upstream_extensions: &upstream_extensions,
        instructions: fields.instructions.as_deref(),
    });
    json!(discovery)
}

pub(crate) fn run_policy(fields: &PolicyFields) -> HelperResult {
    let policy = GatewayVersionPolicy {
        versions: known_versions(&fields.versions),
    };
    let no_params = Object::new();
    let request = if fields.method == "initialize" {
        VersionRequest::Initialize {
            params: fields.params.as_ref().unwrap_or(&no_params),
        }
    } else {
        VersionRequest::Session {
            protocol_version: fields.protocol_version.as_deref(),
        }
    };
    match policy.negotiate(request) {
        Ok(_) => {}
        Err(VersionRejection::Unsupported(rejection)) => {
            return Ok(json!({
                "rejected": {
                    "code": rejection.code,
                    "message": rejection.to_string(),
                    "data": { "supported": rejection.supported },
                }
            }));
        }
        Err(invalid @ VersionRejection::InvalidParams(_)) => {
            return Err(failure("InvalidParams", invalid.to_string()));
        }
    }
    if fields.method != "initialize" {
        return Ok(json!({ "result": { "called": true } }));
    }
    let initialized = serde_json::from_value::<InitializeResult>(json!(fields.initialized))
        .map_err(|error| failure("InvalidInitializeResult", error.to_string()))?;
    Ok(json!({ "result": policy.apply_initialize(initialized) }))
}

pub(crate) fn run_advertised(fields: &AdvertisedFields) -> HelperResult {
    match advertised_versions(fields.configured.as_deref()) {
        Ok(versions) => Ok(json!(
            versions
                .iter()
                .map(|version| version.as_str())
                .collect::<Vec<_>>()
        )),
        Err(error) => Err(failure("ConfigError", error.to_string())),
    }
}
