#![cfg(test)]

use betterlite_db::mcp_stored_contracts::MCPTransport;
use rstest::rstest;

use super::{
    GATEWAY_OPERATIONS, HANDSHAKE_VERSIONS, LATEST_HANDSHAKE_VERSION, ResultKind, SpecVersion,
    revision_support,
};

#[rstest]
#[case::nov_2024(SpecVersion::Nov2024, "2024-11-05")]
#[case::mar_2025(SpecVersion::Mar2025, "2025-03-26")]
#[case::jun_2025(SpecVersion::Jun2025, "2025-06-18")]
#[case::nov_2025(SpecVersion::Nov2025, "2025-11-25")]
#[case::jul_2026(SpecVersion::Jul2026, "2026-07-28")]
fn each_revision_uses_its_wire_text(#[case] version: SpecVersion, #[case] text: &str) {
    assert_eq!(version.as_str(), text);
    assert_eq!(SpecVersion::from_wire(text), Some(version));
}

#[rstest]
#[case::unknown("2099-01-01")]
#[case::empty("")]
#[case::suffix("2025-11-25-rc1")]
fn a_string_outside_the_registry_is_not_a_revision(#[case] text: &str) {
    assert_eq!(SpecVersion::from_wire(text), None);
}

#[rstest]
#[case::nov_2024(SpecVersion::Nov2024)]
#[case::mar_2025(SpecVersion::Mar2025)]
#[case::jun_2025(SpecVersion::Jun2025)]
#[case::nov_2025(SpecVersion::Nov2025)]
fn a_handshake_revision_is_completed_on_every_transport(#[case] version: SpecVersion) {
    assert!(version.is_handshake());
    let support = revision_support(version);
    assert!(support.completed);
    assert_eq!(
        support.transports.to_vec(),
        [MCPTransport::Sse, MCPTransport::Http, MCPTransport::Stdio]
    );
    assert_eq!(support.operations.to_vec(), GATEWAY_OPERATIONS);
    assert_eq!(support.results.to_vec(), [ResultKind::Complete]);
    assert!(support.extensions.is_empty());
}

#[rstest]
fn the_modern_revision_is_uncompleted_http_only_and_allows_input_required() {
    let version = SpecVersion::Jul2026;
    assert!(!version.is_handshake());
    let support = revision_support(version);
    assert!(!support.completed);
    assert_eq!(
        support.transports.to_vec(),
        [MCPTransport::Http, MCPTransport::Stdio]
    );
    assert!(!support.transports.contains(&MCPTransport::Sse));
    assert_eq!(
        support.results.to_vec(),
        [ResultKind::Complete, ResultKind::InputRequired]
    );
    assert_eq!(support.operations.to_vec(), GATEWAY_OPERATIONS);
    assert!(support.extensions.is_empty());
}

#[rstest]
fn the_handshake_versions_are_the_four_legacy_revisions_in_order() {
    assert_eq!(
        HANDSHAKE_VERSIONS,
        [
            SpecVersion::Nov2024,
            SpecVersion::Mar2025,
            SpecVersion::Jun2025,
            SpecVersion::Nov2025
        ]
    );
    assert_eq!(HANDSHAKE_VERSIONS[3].as_str(), LATEST_HANDSHAKE_VERSION);
    assert!(
        HANDSHAKE_VERSIONS
            .iter()
            .all(|version| version.is_handshake())
    );
}

#[rstest]
fn the_gateway_operations_are_the_seven_listing_and_call_methods() {
    assert_eq!(
        GATEWAY_OPERATIONS,
        [
            "tools/list",
            "tools/call",
            "prompts/list",
            "prompts/get",
            "resources/list",
            "resources/read",
            "resources/templates/list"
        ]
    );
}
