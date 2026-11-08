#![cfg(test)]

use rstest::rstest;

use super::{
    ADVERTISED_VERSIONS_FIELD, ConfigError, GatewayVersionPolicy, UnsupportedVersion,
    VersionRequest, advertised_versions,
};
use crate::client::sessions::negotiate::Implementation;
use crate::wire_compat::capabilities::discovery::{
    InitializeResult, ListChangedCapability, ResourcesCapability, ServerCapabilities,
};
use crate::wire_compat::capabilities::versions::{
    HANDSHAKE_VERSIONS, LATEST_HANDSHAKE_VERSION, SpecVersion,
};

fn policy(versions: &[SpecVersion]) -> GatewayVersionPolicy {
    GatewayVersionPolicy {
        versions: versions.to_vec(),
    }
}

fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        tools: Some(ListChangedCapability::default()),
        prompts: Some(ListChangedCapability::default()),
        resources: Some(ResourcesCapability::default()),
        extensions: None,
    }
}

fn initialize_result(protocol_version: &str) -> InitializeResult {
    InitializeResult {
        protocol_version: protocol_version.to_owned(),
        capabilities: capabilities(),
        server_info: Implementation {
            name: "litellm".to_owned(),
            version: "1.105.1".to_owned(),
        },
        instructions: Some("Use the tools.".to_owned()),
    }
}

#[rstest]
#[case::nov_2024("2024-11-05", "2024-11-05")]
#[case::mar_2025("2025-03-26", "2025-03-26")]
#[case::jun_2025("2025-06-18", "2025-06-18")]
#[case::nov_2025("2025-11-25", "2025-11-25")]
#[case::modern("2026-07-28", LATEST_HANDSHAKE_VERSION)]
#[case::unknown("2099-01-01", LATEST_HANDSHAKE_VERSION)]
#[case::empty("", LATEST_HANDSHAKE_VERSION)]
fn an_initialize_offer_negotiates_the_handshake_revision(
    #[case] requested: &str,
    #[case] negotiated: &str,
) {
    let policy = policy(&HANDSHAKE_VERSIONS);
    assert_eq!(
        policy
            .negotiate(VersionRequest::Initialize { requested })
            .unwrap(),
        negotiated
    );
}

#[rstest]
#[case::nov_2024("2024-11-05")]
#[case::nov_2025("2025-11-25")]
fn a_session_keeps_a_configured_revision(#[case] version: &str) {
    let policy = policy(&[SpecVersion::Nov2024, SpecVersion::Nov2025]);
    assert_eq!(
        policy
            .negotiate(VersionRequest::Session {
                protocol_version: Some(version),
            })
            .unwrap(),
        version
    );
}

#[rstest]
fn a_session_revision_outside_the_configuration_is_rejected() {
    let policy = policy(&[SpecVersion::Nov2024]);
    let error = policy
        .negotiate(VersionRequest::Session {
            protocol_version: Some("2026-07-28"),
        })
        .unwrap_err();
    assert_eq!(error.code, -32_022_i32);
}

#[rstest]
fn a_session_without_a_revision_is_rejected() {
    let policy = policy(&[SpecVersion::Nov2025]);
    let error = policy
        .negotiate(VersionRequest::Session {
            protocol_version: None,
        })
        .unwrap_err();
    assert_eq!(error.code, -32_022_i32);
}

#[rstest]
fn an_unconfigured_revision_is_rejected_with_the_python_code_and_supported_list() {
    let policy = policy(&[SpecVersion::Jun2025, SpecVersion::Nov2025]);
    let error = policy
        .negotiate(VersionRequest::Initialize {
            requested: "2024-11-05",
        })
        .unwrap_err();
    assert_eq!(error.code, -32_022_i32);
    assert_eq!(error.to_string(), "Unsupported MCP protocol version");
    assert_eq!(error.supported, ["2025-06-18", "2025-11-25"]);
    assert_eq!(
        error,
        UnsupportedVersion {
            code: -32_022_i32,
            supported: vec!["2025-06-18".to_owned(), "2025-11-25".to_owned()],
        }
    );
}

#[rstest]
fn an_initialize_result_gets_the_discovery_capabilities() {
    let policy = policy(&[SpecVersion::Nov2025]);
    let applied = policy.apply_initialize(initialize_result("2025-11-25"));
    assert_eq!(applied.protocol_version, "2025-11-25");
    assert!(applied.capabilities.tools.is_some());
    assert!(applied.capabilities.prompts.is_some());
    assert!(applied.capabilities.resources.is_some());
    assert_eq!(applied.instructions.as_deref(), Some("Use the tools."));
    assert_eq!(applied.server_info.name, "litellm");
    assert_eq!(applied.server_info.version, "1.105.1");
}

#[rstest]
fn an_unadvertised_initialize_revision_loses_every_capability() {
    let policy = policy(&[SpecVersion::Nov2025]);
    let applied = policy.apply_initialize(initialize_result("2024-11-05"));
    assert_eq!(applied.protocol_version, "2024-11-05");
    assert!(applied.capabilities.tools.is_none());
    assert!(applied.capabilities.prompts.is_none());
    assert!(applied.capabilities.resources.is_none());
}

#[rstest]
fn a_missing_setting_advertises_the_four_legacy_revisions() {
    assert_eq!(
        advertised_versions(None).unwrap(),
        [
            SpecVersion::Nov2024,
            SpecVersion::Mar2025,
            SpecVersion::Jun2025,
            SpecVersion::Nov2025
        ]
    );
}

#[rstest]
fn a_configured_list_is_read_in_order() {
    let configured = ["2025-11-25".to_owned(), "2024-11-05".to_owned()];
    assert_eq!(
        advertised_versions(Some(&configured)).unwrap(),
        [SpecVersion::Nov2025, SpecVersion::Nov2024]
    );
}

#[rstest]
fn an_empty_list_is_an_error_that_names_the_setting() {
    let empty: [String; 0] = [];
    let error = advertised_versions(Some(&empty)).unwrap_err();
    assert_eq!(
        error,
        ConfigError::EmptyAdvertisedVersions {
            field: ADVERTISED_VERSIONS_FIELD
        }
    );
    assert!(
        error
            .to_string()
            .starts_with("general_settings.mcp_advertised_versions")
    );
}

#[rstest]
#[case::modern("2026-07-28")]
#[case::unknown("2099-01-01")]
fn a_non_legacy_version_is_an_error_that_names_the_setting_and_value(#[case] value: &str) {
    let configured = [value.to_owned()];
    let error = advertised_versions(Some(&configured)).unwrap_err();
    assert_eq!(
        error,
        ConfigError::UnknownAdvertisedVersion {
            field: ADVERTISED_VERSIONS_FIELD,
            value: value.to_owned(),
        }
    );
    assert!(error.to_string().contains(value));
}
