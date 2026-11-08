#![cfg(test)]

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;

use super::{
    ADVERTISED_VERSIONS_FIELD, ConfigError, GatewayVersionPolicy, INVALID_PARAMS,
    UnsupportedVersion, VersionRejection, VersionRequest, advertised_versions,
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

fn initialize_params(requested: &str) -> Object {
    let raw = format!(
        r#"{{"protocolVersion":"{requested}","capabilities":{{}},"clientInfo":{{"name":"client","version":"1"}}}}"#
    );
    serde_json::from_str(&raw).unwrap()
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
    let params = initialize_params(requested);
    assert_eq!(
        policy
            .negotiate(VersionRequest::Initialize { params: &params })
            .unwrap(),
        negotiated
    );
}

#[rstest]
#[case::no_params("{}")]
#[case::no_protocol_version(r#"{"capabilities":{},"clientInfo":{"name":"c","version":"1"}}"#)]
#[case::no_capabilities(
    r#"{"protocolVersion":"2025-11-25","clientInfo":{"name":"c","version":"1"}}"#
)]
#[case::no_client_info(r#"{"protocolVersion":"2025-11-25","capabilities":{}}"#)]
#[case::client_info_without_version(
    r#"{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"c"}}"#
)]
#[case::numeric_protocol_version(
    r#"{"protocolVersion":20251125,"capabilities":{},"clientInfo":{"name":"c","version":"1"}}"#
)]
#[case::roots_not_an_object(
    r#"{"protocolVersion":"2025-11-25","capabilities":{"roots":true},"clientInfo":{"name":"c","version":"1"}}"#
)]
#[case::experimental_not_an_object(
    r#"{"protocolVersion":"2025-11-25","capabilities":{"experimental":"x"},"clientInfo":{"name":"c","version":"1"}}"#
)]
#[case::experimental_member_not_an_object(
    r#"{"protocolVersion":"2025-11-25","capabilities":{"experimental":{"a":1}},"clientInfo":{"name":"c","version":"1"}}"#
)]
#[case::client_info_title_not_a_string(
    r#"{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"c","version":"1","title":1}}"#
)]
#[case::client_info_icon_without_src(
    r#"{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"c","version":"1","icons":[{}]}}"#
)]
#[case::meta_not_an_object(
    r#"{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"c","version":"1"},"_meta":"x"}"#
)]
fn an_initialize_with_invalid_params_is_rejected_with_the_invalid_params_code(#[case] raw: &str) {
    let policy = policy(&HANDSHAKE_VERSIONS);
    let params: Object = serde_json::from_str(raw).unwrap();
    let error = policy
        .negotiate(VersionRequest::Initialize { params: &params })
        .unwrap_err();
    assert!(matches!(error, VersionRejection::InvalidParams(_)));
    assert_eq!(error.code(), INVALID_PARAMS);
    assert_eq!(error.code(), -32_602_i32);
}

#[rstest]
fn an_initialize_with_declared_optional_members_and_unknown_keys_is_accepted() {
    let raw = r#"{"protocolVersion":"2025-11-25","capabilities":{"experimental":{"a":{}},"sampling":{"tools":{}},"roots":{"listChanged":true},"extra":1},"clientInfo":{"name":"c","version":"1","title":"C","websiteUrl":"https://c.test","icons":[{"src":"https://c.test/i.png","theme":"dark"}],"other":2},"_meta":{"k":1},"unknown":3}"#;
    let policy = policy(&HANDSHAKE_VERSIONS);
    let params: Object = serde_json::from_str(raw).unwrap();
    let negotiated = policy.negotiate(VersionRequest::Initialize { params: &params });
    assert_eq!(negotiated, Ok("2025-11-25"));
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
#[case::outside_the_configuration(Some("2026-07-28"))]
#[case::missing(None)]
fn a_session_without_a_configured_revision_is_rejected(#[case] protocol_version: Option<&str>) {
    let policy = policy(&[SpecVersion::Nov2024]);
    let error = policy
        .negotiate(VersionRequest::Session { protocol_version })
        .unwrap_err();
    assert_eq!(error.code(), -32_022_i32);
}

#[rstest]
fn an_unconfigured_revision_is_rejected_with_the_python_code_and_supported_list() {
    let policy = policy(&[SpecVersion::Jun2025, SpecVersion::Nov2025]);
    let params = initialize_params("2024-11-05");
    let error = policy
        .negotiate(VersionRequest::Initialize { params: &params })
        .unwrap_err();
    assert_eq!(error.code(), -32_022_i32);
    assert_eq!(error.to_string(), "Unsupported MCP protocol version");
    assert_eq!(
        error,
        VersionRejection::Unsupported(UnsupportedVersion {
            code: -32_022_i32,
            supported: vec!["2025-06-18".to_owned(), "2025-11-25".to_owned()],
        })
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
#[case::missing(None, &[SpecVersion::Nov2024, SpecVersion::Mar2025, SpecVersion::Jun2025, SpecVersion::Nov2025])]
#[case::configured(Some(&["2025-11-25", "2024-11-05"][..]), &[SpecVersion::Nov2025, SpecVersion::Nov2024])]
fn the_advertised_revisions_come_from_the_setting_in_order(
    #[case] configured: Option<&[&str]>,
    #[case] expected: &[SpecVersion],
) {
    let owned = configured.map(|names| {
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    });
    assert_eq!(advertised_versions(owned.as_deref()).unwrap(), expected);
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
