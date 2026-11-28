#![cfg(test)]

use rstest::rstest;

use super::{GetQuery, authorize_get};
use crate::byok_consent::ConsentError;
use crate::byok_consent::test_support::{page_query as query, registry, server};
use crate::redirect_trust::shape::RedirectUriRejected;
use crate::server_record::MCPServer;

fn github() -> MCPServer {
    server(
        r#"{"server_id": "srv-1", "name": "github", "server_name": "GitHub", "transport": "sse", "byok_description": ["Read repositories", "Open pull requests"], "byok_api_key_help_url": "https://github.com/settings/tokens"}"#,
    )
}

#[rstest]
#[case::missing_response_type(GetQuery { response_type: None, ..query() }, ConsentError::ResponseType)]
#[case::wrong_response_type(GetQuery { response_type: Some("token".to_owned()), ..query() }, ConsentError::ResponseType)]
#[case::missing_redirect_uri(GetQuery { redirect_uri: None, ..query() }, ConsentError::RedirectUriRequired)]
#[case::empty_redirect_uri(GetQuery { redirect_uri: Some(String::new()), ..query() }, ConsentError::RedirectUriRequired)]
#[case::missing_code_challenge(GetQuery { code_challenge: None, ..query() }, ConsentError::CodeChallengeRequired)]
#[case::empty_code_challenge(GetQuery { code_challenge: Some(String::new()), ..query() }, ConsentError::CodeChallengeRequired)]
fn a_refused_request_keeps_litellms_status_and_body(
    #[case] query: GetQuery,
    #[case] expected: ConsentError,
) {
    let error = authorize_get(&query, None).unwrap_err();
    assert_eq!(error.status(), expected.status());
    assert_eq!(error.body(), expected.body());
}

#[test]
fn a_query_parameter_sets_its_field() {
    let mut query = GetQuery::default();
    query.set("response_type", "code".to_owned());
    query.set("server_id", "srv-1".to_owned());
    query.set("unknown", "ignored".to_owned());
    assert_eq!(query.response_type.as_deref(), Some("code"));
    assert_eq!(query.server_id.as_deref(), Some("srv-1"));
}

#[test]
fn a_non_loopback_redirect_uri_carries_the_rejection_detail() {
    let query = GetQuery {
        redirect_uri: Some("https://example.com/callback".to_owned()),
        ..query()
    };
    let error = authorize_get(&query, None).unwrap_err();
    let expected = ConsentError::RedirectUri(RedirectUriRejected {
        description: "redirect_uri must use a loopback host (localhost or 127.0.0.0/8).".to_owned(),
        hint: Some(
            "Native MCP clients should register a callback on http://127.0.0.1:<port>/..."
                .to_owned(),
        ),
        redirect_uri: None,
    });
    assert_eq!(error.status(), expected.status());
    assert_eq!(error.body(), expected.body());
}

#[test]
fn the_form_carries_the_request_and_the_default_challenge_method() {
    let html = authorize_get(&query(), None).unwrap();
    assert!(html.contains("name=\"client_id\" value=\"client-1\""));
    assert!(html.contains("name=\"redirect_uri\" value=\"http://127.0.0.1:1456/callback\""));
    assert!(html.contains("name=\"code_challenge\" value=\"challenge\""));
    assert!(html.contains("name=\"code_challenge_method\" value=\"S256\""));
    assert!(html.contains("name=\"state\" value=\"state-1\""));
    assert!(html.contains("name=\"server_id\" value=\"\""));
}

#[test]
fn an_explicit_challenge_method_reaches_the_form() {
    let query = GetQuery {
        code_challenge_method: Some("plain".to_owned()),
        ..query()
    };
    let html = authorize_get(&query, None).unwrap();
    assert!(html.contains("name=\"code_challenge_method\" value=\"plain\""));
}

#[test]
fn an_empty_challenge_method_falls_back_to_s256() {
    let query = GetQuery {
        code_challenge_method: Some(String::new()),
        ..query()
    };
    let html = authorize_get(&query, None).unwrap();
    assert!(html.contains("name=\"code_challenge_method\" value=\"S256\""));
}

#[test]
fn an_absent_server_id_names_the_fallback_server() {
    let html = authorize_get(&query(), None).unwrap();
    assert!(html.contains("Connect MCP Server MCP"));
    assert!(html.contains("class=\"logo logo-s\">M<"));
    assert!(!html.contains("class=\"access-item\""));
    assert!(!html.contains("class=\"help-link\""));
}

#[test]
fn an_unknown_server_id_names_the_fallback_server() {
    let query = GetQuery {
        server_id: Some("srv-missing".to_owned()),
        ..query()
    };
    let html = authorize_get(&query, Some(&registry(vec![github()]))).unwrap();
    assert!(html.contains("Connect MCP Server MCP"));
}

#[test]
fn the_registry_entry_supplies_the_page_metadata() {
    let query = GetQuery {
        server_id: Some("srv-1".to_owned()),
        ..query()
    };
    let html = authorize_get(&query, Some(&registry(vec![github()]))).unwrap();
    assert!(html.contains("Connect GitHub MCP"));
    assert!(html.contains("class=\"logo logo-s\">G<"));
    assert_eq!(html.matches("class=\"access-item\"").count(), 2);
    assert!(html.contains("Read repositories"));
    assert!(html.contains("Open pull requests"));
    assert!(html.contains("class=\"help-link\" href=\"https://github.com/settings/tokens\""));
}

#[test]
fn a_registry_entry_without_a_display_name_uses_its_name() {
    let query = GetQuery {
        server_id: Some("srv-2".to_owned()),
        ..query()
    };
    let plain = server(
        r#"{"server_id": "srv-2", "name": "notion", "transport": "sse", "byok_description": []}"#,
    );
    let html = authorize_get(&query, Some(&registry(vec![plain]))).unwrap();
    assert!(html.contains("Connect notion MCP"));
    assert!(html.contains("class=\"logo logo-s\">N<"));
}
