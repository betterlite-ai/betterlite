#![cfg(test)]

use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;

use super::{
    TargetQuery, is_legacy_delegate_cold_start, is_passthrough_cold_start, is_true_passthrough,
    single_bridge_target,
};

const PASSTHROUGH: &str =
    r#"{"server_id":"srv-pt","name":"pt","transport":"http","auth_type":"true_passthrough"}"#;
const PLAIN: &str = r#"{"server_id":"srv-plain","name":"plain","transport":"http","auth_type":"bearer_token","extra_headers":["authorization"]}"#;
const OAUTH_PASSTHROUGH: &str = r#"{"server_id":"srv-op","name":"op","transport":"http","oauth_passthrough":true,"extra_headers":["authorization"]}"#;
const LEGACY_DELEGATE: &str = r#"{"server_id":"srv-legacy","name":"legacy","transport":"http","auth_type":"oauth2","delegate_auth_to_upstream":true}"#;
const CLIENT_CREDENTIALS_DELEGATE: &str = r#"{"server_id":"srv-legacy-cc","name":"legacy-cc","transport":"http","auth_type":"oauth2","delegate_auth_to_upstream":true,"oauth2_flow":"client_credentials"}"#;
const BRIDGE: &str = r#"{"server_id":"srv-bridge","name":"bridge","transport":"http","auth_type":"oauth_delegate","dcr_bridge":true,"server_name":"bridge-name","alias":"bridge-alias"}"#;
const BRIDGE_NAME_ONLY: &str = r#"{"server_id":"srv-bridge-name","name":"bridge-name-only","transport":"http","auth_type":"oauth_delegate","dcr_bridge":true,"server_name":"bridge-name-only"}"#;
const BRIDGE_ALIAS_ONLY: &str = r#"{"server_id":"srv-bridge-alias","name":"bridge-alias-only","transport":"http","auth_type":"oauth_delegate","dcr_bridge":true,"alias":"bridge-alias-only"}"#;
const BRIDGE_UNNAMED: &str = r#"{"server_id":"srv-bridge-plain","name":"bridge-plain","transport":"http","auth_type":"oauth_delegate","dcr_bridge":true}"#;
const BRIDGE_EMPTY_ALIAS: &str = r#"{"server_id":"srv-bridge-empty","name":"bridge-empty","transport":"http","auth_type":"oauth_delegate","dcr_bridge":true,"alias":""}"#;
const BRIDGE_EMPTY_NAME: &str = r#"{"server_id":"srv-bridge-empty-name","name":"bridge-empty-name","transport":"http","auth_type":"oauth_delegate","dcr_bridge":true,"server_name":""}"#;
const OAUTH2_NO_DELEGATE: &str =
    r#"{"server_id":"srv-oauth2","name":"oauth2-plain","transport":"http","auth_type":"oauth2"}"#;
const BEARER_DELEGATE: &str = r#"{"server_id":"srv-bearer-delegate","name":"bearer-delegate","transport":"http","auth_type":"bearer_token","delegate_auth_to_upstream":true}"#;
const DELEGATE_ONLY: &str = r#"{"server_id":"srv-delegate","name":"delegate","transport":"http","auth_type":"oauth_delegate","server_name":"delegate-name"}"#;

fn server(source: &str) -> MCPServer {
    serde_json::from_str(source).unwrap()
}

fn registry(records: &[&str]) -> ServerRegistry {
    let servers: Vec<MCPServer> = records.iter().copied().map(server).collect();
    ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: true,
            internal_ip_ranges: None,
        },
        Vec::new(),
        servers,
    )
}

fn query<'a>(
    registry: &'a ServerRegistry,
    route: &'a str,
    header_servers: Option<&'a [String]>,
) -> TargetQuery<'a> {
    TargetQuery {
        registry,
        route,
        header_servers,
        client_ip: None,
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().copied().map(str::to_owned).collect()
}

#[test]
fn is_true_passthrough_needs_every_target() {
    let registry = registry(&[PASSTHROUGH, PLAIN]);
    assert!(is_true_passthrough(&query(&registry, "/mcp/pt", None)));
    assert!(!is_true_passthrough(&query(&registry, "/mcp/plain", None)));
    assert!(!is_true_passthrough(&query(
        &registry,
        "/mcp/pt,plain",
        None
    )));
    assert!(!is_true_passthrough(&query(
        &registry,
        "/mcp/missing",
        None
    )));
    assert!(!is_true_passthrough(&query(&registry, "/mcp", None)));
}

#[test]
fn is_true_passthrough_reads_header_targets_on_the_aggregate_route() {
    let registry = registry(&[PASSTHROUGH, PLAIN]);
    let both = names(&["pt", "plain"]);
    let only_passthrough = names(&["pt"]);
    assert!(!is_true_passthrough(&query(&registry, "/mcp", Some(&both))));
    assert!(is_true_passthrough(&query(
        &registry,
        "/mcp",
        Some(&only_passthrough)
    )));
}

#[test]
fn single_bridge_target_finds_the_dcr_bridge_delegate() {
    let registry = registry(&[BRIDGE]);
    let target = single_bridge_target(&query(&registry, "/mcp/bridge-alias", None)).unwrap();
    assert_eq!(target.requested_name, "bridge-alias");
    assert_eq!(target.server.fields.server_id, "srv-bridge");
}

#[test]
fn single_bridge_target_keeps_the_requested_spelling() {
    let registry = registry(&[BRIDGE]);
    let by_alias = single_bridge_target(&query(&registry, "/mcp/bridge-alias", None)).unwrap();
    let by_name = single_bridge_target(&query(&registry, "/bridge-name/mcp", None)).unwrap();
    assert_eq!(by_alias.requested_name, "bridge-alias");
    assert_eq!(by_name.requested_name, "bridge-name");
}

#[test]
fn single_bridge_target_refuses_non_bridge_servers() {
    let registry = registry(&[BRIDGE, PASSTHROUGH, DELEGATE_ONLY]);
    assert!(single_bridge_target(&query(&registry, "/mcp/pt", None)).is_none());
    assert!(single_bridge_target(&query(&registry, "/mcp/delegate", None)).is_none());
    assert!(single_bridge_target(&query(&registry, "/mcp/missing", None)).is_none());
}

#[test]
fn single_bridge_target_refuses_more_than_one_target() {
    let registry = registry(&[BRIDGE, PASSTHROUGH]);
    let both = names(&["bridge-alias", "pt"]);
    assert!(single_bridge_target(&query(&registry, "/mcp/bridge-alias,pt", None)).is_none());
    assert!(single_bridge_target(&query(&registry, "/mcp", Some(&both))).is_none());
}

#[test]
fn single_bridge_target_accepts_either_egress_name() {
    let registry = registry(&[BRIDGE_NAME_ONLY, BRIDGE_ALIAS_ONLY]);
    let by_name = single_bridge_target(&query(&registry, "/mcp/bridge-name-only", None)).unwrap();
    let by_alias = single_bridge_target(&query(&registry, "/mcp/bridge-alias-only", None)).unwrap();
    assert_eq!(by_name.requested_name, "bridge-name-only");
    assert_eq!(by_alias.requested_name, "bridge-alias-only");
}

#[test]
fn single_bridge_target_refuses_a_server_without_an_egress_name() {
    let registry = registry(&[BRIDGE_UNNAMED, BRIDGE_EMPTY_ALIAS, BRIDGE_EMPTY_NAME]);
    assert!(single_bridge_target(&query(&registry, "/mcp/bridge-plain", None)).is_none());
    assert!(single_bridge_target(&query(&registry, "/mcp/bridge-empty", None)).is_none());
    assert!(single_bridge_target(&query(&registry, "/mcp/bridge-empty-name", None)).is_none());
}

#[test]
fn passthrough_cold_start_needs_every_name_passthrough() {
    let registry = registry(&[OAUTH_PASSTHROUGH, PLAIN, LEGACY_DELEGATE]);
    let one = names(&["op"]);
    assert!(is_passthrough_cold_start(
        &query(&registry, "/mcp", None),
        &one
    ));
    assert!(!is_passthrough_cold_start(
        &query(&registry, "/mcp", None),
        &[]
    ));
    assert!(!is_passthrough_cold_start(
        &query(&registry, "/mcp", None),
        &names(&["missing"])
    ));
    assert!(!is_passthrough_cold_start(
        &query(&registry, "/mcp", None),
        &names(&["op", "plain"])
    ));
    assert!(!is_passthrough_cold_start(
        &query(&registry, "/mcp", None),
        &names(&["legacy"])
    ));
}

#[test]
fn legacy_delegate_cold_start_follows_the_effective_flow() {
    let registry = registry(&[
        LEGACY_DELEGATE,
        CLIENT_CREDENTIALS_DELEGATE,
        OAUTH2_NO_DELEGATE,
        BEARER_DELEGATE,
        PLAIN,
    ]);
    let delegate = names(&["legacy"]);
    assert!(is_legacy_delegate_cold_start(
        &query(&registry, "/mcp", None),
        &delegate
    ));
    assert!(!is_legacy_delegate_cold_start(
        &query(&registry, "/mcp", None),
        &[]
    ));
    assert!(!is_legacy_delegate_cold_start(
        &query(&registry, "/mcp", None),
        &names(&["oauth2-plain"])
    ));
    assert!(!is_legacy_delegate_cold_start(
        &query(&registry, "/mcp", None),
        &names(&["bearer-delegate"])
    ));
    assert!(!is_legacy_delegate_cold_start(
        &query(&registry, "/mcp", None),
        &names(&["missing"])
    ));
    assert!(!is_legacy_delegate_cold_start(
        &query(&registry, "/mcp", None),
        &names(&["plain"])
    ));
    assert!(!is_legacy_delegate_cold_start(
        &query(&registry, "/mcp", None),
        &names(&["legacy", "legacy-cc"])
    ));
}
