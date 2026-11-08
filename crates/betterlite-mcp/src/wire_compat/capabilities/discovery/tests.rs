#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::MCPTransport;
use betterlite_prompts::arguments::json::Object;
use rstest::rstest;

use super::{
    CacheScope, DiscoverResult, DiscoveryRequest, ListChangedCapability, ResourcesCapability,
    ServerCapabilities, build_discovery,
};
use crate::wire_compat::capabilities::versions::{
    GATEWAY_OPERATIONS, HANDSHAKE_VERSIONS, ResultKind, SpecVersion,
};

fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        tools: Some(ListChangedCapability::default()),
        prompts: Some(ListChangedCapability::default()),
        resources: Some(ResourcesCapability::default()),
        extensions: Some(BTreeMap::from([(
            "io.modelcontextprotocol/ui".to_owned(),
            Object::new(),
        )])),
    }
}

fn discovery(
    configured: &[SpecVersion],
    revision: Option<SpecVersion>,
    transport: MCPTransport,
    capabilities: ServerCapabilities,
) -> DiscoverResult {
    build_discovery(DiscoveryRequest {
        configured,
        revision,
        transport,
        authorized_operations: &GATEWAY_OPERATIONS,
        upstream_versions: &HANDSHAKE_VERSIONS,
        capabilities,
        client_extensions: &[],
        upstream_extensions: &[],
        instructions: None,
    })
}

fn discovery_for_operations(operations: &[&str]) -> DiscoverResult {
    build_discovery(DiscoveryRequest {
        configured: &[SpecVersion::Nov2025],
        revision: Some(SpecVersion::Nov2025),
        transport: MCPTransport::Http,
        authorized_operations: operations,
        upstream_versions: &HANDSHAKE_VERSIONS,
        capabilities: capabilities(),
        client_extensions: &[],
        upstream_extensions: &[],
        instructions: None,
    })
}

#[rstest]
#[case::nov_2024(SpecVersion::Nov2024)]
#[case::mar_2025(SpecVersion::Mar2025)]
#[case::jun_2025(SpecVersion::Jun2025)]
#[case::nov_2025(SpecVersion::Nov2025)]
fn a_supported_revision_gets_the_full_discovery_answer(#[case] revision: SpecVersion) {
    let result = discovery(
        &[revision],
        Some(revision),
        MCPTransport::Http,
        capabilities(),
    );
    assert_eq!(result.supported_versions, [revision.as_str().to_owned()]);
    assert!(result.capabilities.tools.is_some());
    assert!(result.capabilities.prompts.is_some());
    assert!(result.capabilities.resources.is_some());
    assert!(result.capabilities.extensions.is_none());
    assert_eq!(result.result_type, ResultKind::Complete);
    assert_eq!(result.cache_scope, CacheScope::Private);
    assert_eq!(result.ttl_ms, 0);
    assert!(result.instructions.is_none());
}

#[rstest]
#[case::http(MCPTransport::Http)]
#[case::sse(MCPTransport::Sse)]
#[case::stdio(MCPTransport::Stdio)]
fn every_transport_carries_a_completed_revision(#[case] transport: MCPTransport) {
    let result = discovery(
        &[SpecVersion::Nov2025],
        Some(SpecVersion::Nov2025),
        transport,
        capabilities(),
    );
    assert_eq!(result.supported_versions, ["2025-11-25".to_owned()]);
}

#[rstest]
fn the_modern_revision_is_configured_but_never_supported() {
    let result = discovery(
        &SpecVersion::ALL,
        Some(SpecVersion::Jul2026),
        MCPTransport::Http,
        capabilities(),
    );
    assert_eq!(
        result.supported_versions,
        [
            "2024-11-05".to_owned(),
            "2025-03-26".to_owned(),
            "2025-06-18".to_owned(),
            "2025-11-25".to_owned()
        ]
    );
    assert!(result.capabilities.tools.is_none());
    assert!(result.capabilities.prompts.is_none());
    assert!(result.capabilities.resources.is_none());
}

#[rstest]
fn a_configuration_of_only_the_modern_revision_advertises_nothing() {
    let result = discovery(
        &[SpecVersion::Jul2026],
        Some(SpecVersion::Jul2026),
        MCPTransport::Http,
        capabilities(),
    );
    assert!(result.supported_versions.is_empty());
    assert!(result.capabilities.tools.is_none());
}

#[rstest]
#[case::outside_the_configuration(Some(SpecVersion::Nov2024))]
#[case::unknown(None)]
fn an_unconfigured_revision_gains_no_capabilities(#[case] revision: Option<SpecVersion>) {
    let result = discovery(
        &[SpecVersion::Nov2025],
        revision,
        MCPTransport::Http,
        capabilities(),
    );
    assert_eq!(result.supported_versions, ["2025-11-25".to_owned()]);
    assert!(result.capabilities.tools.is_none());
    assert!(result.capabilities.prompts.is_none());
    assert!(result.capabilities.resources.is_none());
}

const NO_UPSTREAM: [SpecVersion; 0] = [];
const MODERN_UPSTREAM: [SpecVersion; 1] = [SpecVersion::Jul2026];

#[rstest]
#[case::no_upstream_versions(&NO_UPSTREAM)]
#[case::modern_upstream(&MODERN_UPSTREAM)]
fn translation_needs_a_completed_upstream_revision(#[case] upstream: &[SpecVersion]) {
    let result = build_discovery(DiscoveryRequest {
        configured: &[SpecVersion::Nov2025],
        revision: Some(SpecVersion::Nov2025),
        transport: MCPTransport::Http,
        authorized_operations: &GATEWAY_OPERATIONS,
        upstream_versions: upstream,
        capabilities: capabilities(),
        client_extensions: &[],
        upstream_extensions: &[],
        instructions: None,
    });
    assert!(result.capabilities.tools.is_none());
    assert!(result.capabilities.prompts.is_none());
    assert!(result.capabilities.resources.is_none());
}

#[rstest]
#[case::tools_list_only(&["tools/list"], [false, false, false])]
#[case::tools_call_only(&["tools/call"], [false, false, false])]
#[case::tools_both(&["tools/list", "tools/call"], [true, false, false])]
#[case::prompts_list_only(&["prompts/list"], [false, false, false])]
#[case::prompts_get_only(&["prompts/get"], [false, false, false])]
#[case::prompts_both(&["prompts/list", "prompts/get"], [false, true, false])]
#[case::resources_list_only(&["resources/list"], [false, false, false])]
#[case::resources_read_only(&["resources/read"], [false, false, false])]
#[case::resources_both(&["resources/list", "resources/read"], [false, false, true])]
fn a_capability_needs_both_of_its_operations_authorized(
    #[case] operations: &[&str],
    #[case] granted: [bool; 3],
) {
    let result = discovery_for_operations(operations);
    assert_eq!(
        [
            result.capabilities.tools.is_some(),
            result.capabilities.prompts.is_some(),
            result.capabilities.resources.is_some(),
        ],
        granted
    );
}

#[rstest]
fn an_extension_offered_by_both_sides_is_still_not_granted() {
    let result = build_discovery(DiscoveryRequest {
        configured: &[SpecVersion::Nov2025],
        revision: Some(SpecVersion::Nov2025),
        transport: MCPTransport::Http,
        authorized_operations: &GATEWAY_OPERATIONS,
        upstream_versions: &HANDSHAKE_VERSIONS,
        capabilities: capabilities(),
        client_extensions: &["io.modelcontextprotocol/ui"],
        upstream_extensions: &["io.modelcontextprotocol/ui"],
        instructions: None,
    });
    assert!(result.capabilities.extensions.is_none());
    assert!(result.capabilities.tools.is_some());
}

#[rstest]
fn instructions_are_copied_to_the_discovery_answer() {
    let result = build_discovery(DiscoveryRequest {
        configured: &[SpecVersion::Nov2025],
        revision: Some(SpecVersion::Nov2025),
        transport: MCPTransport::Http,
        authorized_operations: &GATEWAY_OPERATIONS,
        upstream_versions: &HANDSHAKE_VERSIONS,
        capabilities: capabilities(),
        client_extensions: &[],
        upstream_extensions: &[],
        instructions: Some("Use the tools."),
    });
    assert_eq!(result.instructions.as_deref(), Some("Use the tools."));
}

#[rstest]
fn the_discovery_answer_serializes_with_the_sdk_field_names() {
    let result = discovery(
        &[SpecVersion::Nov2025],
        Some(SpecVersion::Nov2025),
        MCPTransport::Http,
        capabilities(),
    );
    let wire = serde_json::to_string(&result).unwrap();
    assert_eq!(
        wire,
        r#"{"supportedVersions":["2025-11-25"],"capabilities":{"tools":{},"prompts":{},"resources":{}},"resultType":"complete","cacheScope":"private","ttlMs":0}"#
    );
    assert_eq!(
        serde_json::from_str::<DiscoverResult>(&wire).unwrap(),
        result
    );
}
