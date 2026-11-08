use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::MCPTransport;
use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

use crate::client::sessions::negotiate::Implementation;

use super::versions::{ResultKind, SpecVersion, revision_support};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheScope {
    Public,
    Private,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListChangedCapability {
    #[serde(
        rename = "listChanged",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub list_changed: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourcesCapability {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
    #[serde(
        rename = "listChanged",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub list_changed: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ListChangedCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<ListChangedCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, Object>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiscoverResult {
    #[serde(rename = "supportedVersions")]
    pub supported_versions: Vec<String>,
    pub capabilities: ServerCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(rename = "resultType")]
    pub result_type: ResultKind,
    #[serde(rename = "cacheScope")]
    pub cache_scope: CacheScope,
    #[serde(rename = "ttlMs")]
    pub ttl_ms: u64,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Object>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: Implementation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

pub struct DiscoveryRequest<'request> {
    pub configured: &'request [SpecVersion],
    pub revision: Option<SpecVersion>,
    pub transport: MCPTransport,
    pub authorized_operations: &'request [&'request str],
    pub upstream_versions: &'request [SpecVersion],
    pub capabilities: ServerCapabilities,
    pub client_extensions: &'request [&'request str],
    pub upstream_extensions: &'request [&'request str],
    pub instructions: Option<&'request str>,
}

#[must_use]
pub fn build_discovery(request: DiscoveryRequest<'_>) -> DiscoverResult {
    let supported = supported_versions(&request);
    let (supported_operations, supported_extensions) = supported_surface(&request, &supported);
    let operations: Vec<&str> = request
        .authorized_operations
        .iter()
        .copied()
        .filter(|operation| supported_operations.contains(operation))
        .collect();
    DiscoverResult {
        supported_versions: supported
            .iter()
            .map(|version| version.as_str().to_owned())
            .collect(),
        capabilities: offered_capabilities(
            request.capabilities,
            &operations,
            supported_extensions,
            request.client_extensions,
            request.upstream_extensions,
        ),
        instructions: request.instructions.map(str::to_owned),
        result_type: ResultKind::Complete,
        cache_scope: CacheScope::Private,
        ttl_ms: 0,
        meta: None,
    }
}

fn supported_versions(request: &DiscoveryRequest<'_>) -> Vec<SpecVersion> {
    SpecVersion::ALL
        .into_iter()
        .filter(|version| {
            request.configured.contains(version)
                && revision_support(*version).completed
                && revision_support(*version)
                    .transports
                    .contains(&request.transport)
        })
        .collect()
}

fn supported_surface<'request>(
    request: &DiscoveryRequest<'request>,
    supported: &[SpecVersion],
) -> (&'request [&'request str], &'request [&'request str]) {
    let translates = request.revision.is_some_and(|revision| {
        supported.contains(&revision)
            && request
                .upstream_versions
                .iter()
                .any(|upstream| revision.is_handshake() && upstream.is_handshake())
    });
    match request.revision.map(revision_support) {
        Some(support) if translates => (support.operations, support.extensions),
        Some(_) | None => (&[], &[]),
    }
}

fn offered_capabilities(
    capabilities: ServerCapabilities,
    operations: &[&str],
    supported_extensions: &[&str],
    client_extensions: &[&str],
    upstream_extensions: &[&str],
) -> ServerCapabilities {
    let mut offered = ServerCapabilities::default();
    if operations.contains(&"tools/list") && operations.contains(&"tools/call") {
        offered.tools = capabilities.tools;
    }
    if operations.contains(&"prompts/list") && operations.contains(&"prompts/get") {
        offered.prompts = capabilities.prompts;
    }
    if operations.contains(&"resources/list") && operations.contains(&"resources/read") {
        offered.resources = capabilities.resources;
    }
    offered.extensions = capabilities.extensions.and_then(|extensions| {
        let kept: BTreeMap<String, Object> = if operations.is_empty() {
            BTreeMap::new()
        } else {
            extensions
                .into_iter()
                .filter(|(key, _)| {
                    let name = key.as_str();
                    supported_extensions.contains(&name)
                        && client_extensions.contains(&name)
                        && upstream_extensions.contains(&name)
                })
                .collect()
        };
        if kept.is_empty() { None } else { Some(kept) }
    });
    offered
}

mod tests;
