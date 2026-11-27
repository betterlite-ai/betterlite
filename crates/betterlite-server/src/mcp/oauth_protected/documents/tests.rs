use axum::http::StatusCode;
use betterlite_mcp::server_record::MCPServer;
use rstest::rstest;

use super::errors::DiscoveryFailure;
use super::{
    NamedDocument, aggregate_auth_server, aggregate_protected_resource, merge_issuers,
    root_resource,
};
use crate::mcp::oauth_protected::tests::support::{
    AGGREGATE_AUTH_SERVER_DOCUMENT, AGGREGATE_AUTH_SERVER_WITH_EXCHANGE_DOCUMENT,
    AGGREGATE_RESOURCE_DOCUMENT, BASE_URL, GATEWAY_STANDARD_DOCUMENT, Harness, PASS_THROUGH_HOST,
    PASS_THROUGH_PATH, ROOT_DOCUMENT, UPSTREAM_DOCUMENT, dcr_bridge_server, delegate_server,
    gateway_server, keyed_server, legacy_passthrough_server, obo_server, passthrough_server,
    relay_server,
};

async fn named(
    harness: &Harness,
    server: &MCPServer,
    standard: bool,
) -> Result<NamedDocument, DiscoveryFailure> {
    harness
        .root()
        .named_resource(BASE_URL, &server.fields.name, standard, Some(server))
        .await
}

fn text(document: &NamedDocument) -> String {
    serde_json::to_string(document).expect("the named document serializes")
}

#[test]
fn the_root_document_writes_resource_first() {
    let document = root_resource(BASE_URL);
    assert_eq!(
        serde_json::to_string(&document).expect("the root document serializes"),
        ROOT_DOCUMENT
    );
}

#[test]
fn the_root_document_drops_the_path_and_lowercases_the_scheme() {
    let document = root_resource("HTTPS://GW.Example.TEST:8443/base/");
    assert_eq!(
        serde_json::to_string(&document).expect("the root document serializes"),
        r#"{"resource":"https://GW.Example.TEST:8443","authorization_servers":["HTTPS://GW.Example.TEST:8443/base//mcp"],"scopes_supported":[]}"#
    );
}

#[test]
fn a_root_resource_without_a_scheme_keeps_the_url() {
    let document = root_resource("litellm.internal");
    assert_eq!(
        serde_json::to_string(&document).expect("the root document serializes"),
        r#"{"resource":"litellm.internal","authorization_servers":["litellm.internal/mcp"],"scopes_supported":[]}"#
    );
}

#[test]
fn the_aggregate_protected_resource_writes_authorization_servers_first() {
    let document = aggregate_protected_resource(BASE_URL);
    assert_eq!(
        serde_json::to_string(&document).expect("the aggregate document serializes"),
        AGGREGATE_RESOURCE_DOCUMENT
    );
}

#[rstest]
#[case::without_token_exchange(false, AGGREGATE_AUTH_SERVER_DOCUMENT)]
#[case::with_token_exchange(true, AGGREGATE_AUTH_SERVER_WITH_EXCHANGE_DOCUMENT)]
fn the_aggregate_authorization_server_lists_the_grants(
    #[case] token_exchange_available: bool,
    #[case] expected: &str,
) {
    let document = aggregate_auth_server(BASE_URL, token_exchange_available);
    assert_eq!(
        serde_json::to_string(&document).expect("the authorization server document serializes"),
        expected
    );
}

#[test]
fn merges_the_env_issuer_before_the_configured_ones() {
    let configured = [
        Some(String::from("https://idp-a.example.test")),
        None,
        Some(String::from("https://idp-env.example.test")),
        Some(String::new()),
        Some(String::from("https://idp-b.example.test")),
    ];
    assert_eq!(
        merge_issuers(Some("https://idp-env.example.test"), &configured),
        vec![
            String::from("https://idp-env.example.test"),
            String::from("https://idp-a.example.test"),
            String::from("https://idp-b.example.test"),
        ]
    );
}

#[test]
fn an_empty_env_issuer_is_skipped() {
    let configured = [Some(String::from("https://idp-a.example.test"))];
    assert_eq!(
        merge_issuers(Some(""), &configured),
        vec![String::from("https://idp-a.example.test")]
    );
}

#[test]
fn no_issuers_merge_to_an_empty_list() {
    assert!(merge_issuers(None, &[]).is_empty());
}

#[rstest]
#[case::standard(true, r#"{"authorization_servers":["http://litellm.internal/bridge"],"resource":"http://litellm.internal/mcp/bridge","scopes_supported":["read"]}"#)]
#[case::legacy(false, r#"{"authorization_servers":["http://litellm.internal/bridge"],"resource":"http://litellm.internal/bridge/mcp","scopes_supported":["read"]}"#)]
#[tokio::test]
async fn a_dcr_bridge_advertises_the_per_server_authorization_server(
    #[case] standard: bool,
    #[case] expected: &str,
) {
    let harness = Harness::new();
    let document = named(&harness, &dcr_bridge_server(), standard)
        .await
        .expect("the bridge document builds");
    assert_eq!(text(&document), expected);
}

#[tokio::test]
async fn the_legacy_passthrough_rewrites_the_resource() {
    let harness = Harness::new().reply(PASS_THROUGH_HOST, UPSTREAM_DOCUMENT);
    let document = named(&harness, &legacy_passthrough_server(), true)
        .await
        .expect("the relayed document builds");
    assert_eq!(
        text(&document),
        r#"{"resource":"http://litellm.internal/mcp/legacy","authorization_servers":["https://idp.example.test"],"scopes_supported":["mcp"]}"#
    );
}

#[tokio::test]
async fn a_document_without_a_resource_key_gets_it_appended() {
    let harness = Harness::new().reply(
        PASS_THROUGH_HOST,
        r#"{"authorization_servers":["https://idp.example.test"]}"#,
    );
    let document = named(&harness, &legacy_passthrough_server(), false)
        .await
        .expect("the relayed document builds");
    assert_eq!(
        text(&document),
        r#"{"authorization_servers":["https://idp.example.test"],"resource":"http://litellm.internal/legacy/mcp"}"#
    );
}

#[tokio::test]
async fn a_true_passthrough_keeps_the_upstream_resource() {
    let harness = Harness::new().reply(PASS_THROUGH_HOST, UPSTREAM_DOCUMENT);
    let document = named(&harness, &passthrough_server(), true)
        .await
        .expect("the relayed document builds");
    assert_eq!(text(&document), UPSTREAM_DOCUMENT);
}

#[tokio::test]
async fn an_oauth_delegate_keeps_the_upstream_resource() {
    let harness = Harness::new().reply(PASS_THROUGH_HOST, UPSTREAM_DOCUMENT);
    let document = named(&harness, &delegate_server(), true)
        .await
        .expect("the relayed document builds");
    assert_eq!(text(&document), UPSTREAM_DOCUMENT);
}

#[tokio::test]
async fn an_unavailable_upstream_is_a_502() {
    let harness = Harness::new()
        .status(PASS_THROUGH_HOST, 404)
        .status(PASS_THROUGH_PATH, 404);
    let failure = named(&harness, &passthrough_server(), true)
        .await
        .expect_err("no upstream document is not a document");
    assert_eq!(failure.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        failure.detail,
        "Upstream oauth-protected-resource metadata unavailable for MCP server 'upstream'"
    );
}

#[tokio::test]
async fn a_failed_upstream_fetch_is_a_502() {
    let harness = Harness::new()
        .transport_failure(PASS_THROUGH_HOST, "connection refused")
        .transport_failure(PASS_THROUGH_PATH, "connection refused");
    let failure = named(&harness, &passthrough_server(), true)
        .await
        .expect_err("a transport failure is not a document");
    assert_eq!(failure.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        failure.detail,
        "Failed to fetch upstream oauth-protected-resource metadata for MCP server 'upstream'"
    );
}

#[tokio::test]
async fn the_502_detail_quotes_a_name_with_an_apostrophe_with_double_quotes() {
    let mut server = passthrough_server();
    server.fields.name = String::from("it's");
    let harness = Harness::new()
        .status(PASS_THROUGH_HOST, 404)
        .status(PASS_THROUGH_PATH, 404);
    let failure = named(&harness, &server, true)
        .await
        .expect_err("no upstream document is not a document");
    assert_eq!(
        failure.detail,
        r#"Upstream oauth-protected-resource metadata unavailable for MCP server "it's""#
    );
}

#[tokio::test]
async fn an_unknown_server_and_a_keyed_server_share_the_404() {
    let harness = Harness::new();
    let unknown = harness
        .root()
        .named_resource(BASE_URL, "ghost", true, None)
        .await
        .expect_err("an unknown server is refused");
    let keyed = named(&harness, &keyed_server(), true)
        .await
        .expect_err("a keyed server is refused");
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(keyed.status, StatusCode::NOT_FOUND);
    assert_eq!(
        unknown.detail,
        "MCP server 'ghost' is not an OAuth-protected resource"
    );
    assert_eq!(
        keyed.detail,
        "MCP server 'keyed' is not an OAuth-protected resource"
    );
}

#[tokio::test]
async fn an_obo_server_advertises_the_jwt_issuers() {
    let harness =
        Harness::new().issuers(&["https://idp-a.example.test", "https://idp-b.example.test"]);
    let document = named(&harness, &obo_server(), true)
        .await
        .expect("the OBO document builds");
    assert_eq!(
        text(&document),
        r#"{"authorization_servers":["https://idp-a.example.test","https://idp-b.example.test"],"resource":"http://litellm.internal/mcp/obo","scopes_supported":["read"]}"#
    );
}

#[tokio::test]
async fn an_obo_server_without_issuers_falls_back_to_the_per_server_path() {
    let harness = Harness::new();
    let document = named(&harness, &obo_server(), true)
        .await
        .expect("the OBO fallback document builds");
    assert_eq!(
        text(&document),
        r#"{"authorization_servers":["http://litellm.internal/obo"],"resource":"http://litellm.internal/mcp/obo","scopes_supported":["read"]}"#
    );
}

#[tokio::test]
async fn an_oauth2_server_advertises_the_gateway() {
    let harness = Harness::new();
    let document = named(&harness, &gateway_server(), true)
        .await
        .expect("the gateway document builds");
    assert_eq!(text(&document), GATEWAY_STANDARD_DOCUMENT);
}

#[tokio::test]
async fn a_relay_server_advertises_its_own_path() {
    let harness = Harness::new();
    let document = named(&harness, &relay_server(), false)
        .await
        .expect("the relay document builds");
    assert_eq!(
        text(&document),
        r#"{"authorization_servers":["http://litellm.internal/test-oauth-1"],"resource":"http://litellm.internal/test-oauth-1/mcp","scopes_supported":[]}"#
    );
}
