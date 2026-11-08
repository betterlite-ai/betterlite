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
    instructions: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolicyFields {
    versions: Vec<String>,
    method: String,
    params: Option<Object>,
    protocol_version: Option<String>,
    initialized: Object,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CapabilityInput {
    Discovery(DiscoveryFields),
    VersionPolicy(PolicyFields),
    AdvertisedVersions { configured: Option<Vec<String>> },
}

pub(crate) fn run(input: &CapabilityInput) -> HelperResult {
    match input {
        CapabilityInput::Discovery(fields) => Ok(run_discovery(fields)),
        CapabilityInput::VersionPolicy(fields) => run_policy(fields),
        CapabilityInput::AdvertisedVersions { configured } => run_advertised(configured.as_deref()),
    }
}

fn known_versions(wire: &[String]) -> Vec<SpecVersion> {
    wire.iter()
        .filter_map(|version| SpecVersion::from_wire(version))
        .collect()
}

fn run_discovery(fields: &DiscoveryFields) -> Value {
    let [operations, client_extensions, upstream_extensions] = [
        &fields.authorized_operations,
        &fields.client_extensions,
        &fields.upstream_extensions,
    ]
    .map(|names| names.iter().map(String::as_str).collect::<Vec<_>>());
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

fn run_policy(fields: &PolicyFields) -> HelperResult {
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
        Ok(_) if fields.method != "initialize" => {
            return Ok(json!({ "result": { "called": true } }));
        }
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
        Err(VersionRejection::InvalidParams(message)) => {
            return Err(failure("InvalidParams", message));
        }
    }
    let initialized = serde_json::from_value::<InitializeResult>(json!(fields.initialized))
        .map_err(|error| failure("InvalidInitializeResult", error.to_string()))?;
    Ok(json!({ "result": policy.apply_initialize(initialized) }))
}

fn run_advertised(configured: Option<&[String]>) -> HelperResult {
    advertised_versions(configured)
        .map(|list| list.into_iter().map(SpecVersion::as_str).collect::<Value>())
        .map_err(|error| failure("ConfigError", error.to_string()))
}
