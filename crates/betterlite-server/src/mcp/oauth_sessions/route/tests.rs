#![cfg(test)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::{Principal, SourceKey};
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use rstest::rstest;
use tower::ServiceExt as _;

use super::{
    FORBIDDEN_MESSAGE, SESSION_PATH, add_session_server, authenticate, management_failure,
};
use crate::auth::auth_identity::AuthContext;
use crate::mcp::oauth_sessions::SessionError;
use crate::mcp::oauth_sessions::tests::testing::{
    AccessFailure, DatabaseFault, HarnessConfig, auth_with, harness, row, server, session_route,
};

const SESSION_BODY: &str = r#"{
    "server_id":"test-session-1",
    "server_name":"Session",
    "url":"https://mcp.example.test/sse",
    "transport":"sse",
    "auth_type":"oauth2",
    "credentials":{
        "client_id":"abc",
        "client_secret":"client-secret",
        "auth_value":"auth-token",
        "upstream_resource":"https://resource.example.test",
        "scopes":["scope-a"]
    }
}"#;

#[tokio::test]
async fn an_admin_creates_a_redacted_session() {
    let harness = harness(HarnessConfig::new());
    let route = session_route(&harness);
    let auth = auth_with(Some("admin-1"), "proxy_admin");
    let Ok(record) = add_session_server(&route, SESSION_BODY.as_bytes(), &auth).await else {
        panic!("the admin creates the session");
    };
    assert_eq!(record.server_id, "test-session-1");
    assert_eq!(record.server_name.as_deref(), Some("Session"));
    assert_eq!(record.alias.as_deref(), Some("Session"));
    assert_eq!(record.created_by.as_deref(), Some("admin-1"));
    assert_eq!(record.updated_by.as_deref(), Some("admin-1"));
    assert_redacted_credentials(&record);
    let info = record
        .mcp_info
        .as_ref()
        .expect("the view carries the mcp info");
    let encoded = serde_json::to_string(info).expect("the info encodes");
    assert!(encoded.contains("\"is_public\":false"));
    assert_eq!(
        harness.builder.calls(),
        vec![("test-session-1".to_owned(), false)]
    );
    let cached = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the local lookup has no error")
        .expect("the session is cached locally");
    assert_eq!(cached.fields.server_id, "test-session-1");
}

#[tokio::test]
async fn the_router_answers_an_admin_over_http() {
    let harness = harness(HarnessConfig::new());
    let route = session_route(&harness);
    let auth = auth_with(Some("admin-1"), "proxy_admin");
    let mut principal: Principal = serde_json::from_str(
        r#"{"principal_type":"human","subject":"admin-1","roles":[],"auth_method":"api_key"}"#,
    )
    .expect("the test principal parses");
    principal.source_key = SourceKey(Some(auth));
    let request = Request::post(SESSION_PATH)
        .header("litellm-changed-by", "admin-2")
        .extension(AuthContext { principal })
        .body(Body::from(SESSION_BODY))
        .expect("the request builds");
    let response = route
        .router()
        .oneshot(request)
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .expect("the response body reads");
    let text = String::from_utf8(body.to_vec()).expect("the body is utf-8");
    assert!(text.contains(r#""server_id":"test-session-1""#));
    assert!(!text.contains("client-secret"));
}

// the admin view keeps the scopes and the upstream resource, and drops every secret
fn assert_redacted_credentials(record: &McpServerTable) {
    let credentials = record
        .credentials
        .as_ref()
        .expect("the admin config credentials survive");
    assert_eq!(
        credentials.upstream_resource,
        Some(Some("https://resource.example.test".to_owned()))
    );
    assert_eq!(credentials.scopes, Some(Some(vec!["scope-a".to_owned()])));
    assert!(credentials.auth_value.is_none());
    assert!(credentials.client_id.is_none());
    assert!(credentials.client_secret.is_none());
}

#[tokio::test]
async fn a_request_without_an_id_gets_a_fresh_session_id() {
    let harness = harness(HarnessConfig::new());
    let route = session_route(&harness);
    let auth = auth_with(Some("admin-1"), "proxy_admin");
    let body = r#"{"url":"https://mcp.example.test/sse","transport":"sse"}"#;
    let Ok(record) = add_session_server(&route, body.as_bytes(), &auth).await else {
        panic!("the admin creates the session");
    };
    assert_eq!(record.server_id, "fresh-session-1");
    assert_eq!(record.server_name.as_deref(), Some("fresh-session-1"));
}

#[tokio::test]
async fn a_supplied_id_that_the_registry_knows_is_kept() {
    let harness = harness(HarnessConfig::new().with_servers(vec![server("saved-1")]));
    let route = session_route(&harness);
    let auth = auth_with(Some("admin-1"), "proxy_admin");
    let body = r#"{"server_id":"saved-1","url":"https://mcp.example.test/sse","transport":"sse"}"#;
    let Ok(record) = add_session_server(&route, body.as_bytes(), &auth).await else {
        panic!("the admin creates the session");
    };
    assert_eq!(record.server_id, "saved-1");
}

#[tokio::test]
async fn a_bad_server_name_answers_400_before_the_role_check() {
    let harness = harness(HarnessConfig::new());
    let route = session_route(&harness);
    let auth = auth_with(Some("user-1"), "internal_user");
    let body =
        r#"{"url":"https://mcp.example.test/sse","transport":"sse","server_name":"bad-name"}"#;
    let Err(error) = add_session_server(&route, body.as_bytes(), &auth).await else {
        panic!("a separator in the name is rejected");
    };
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(matches!(error, SessionError::Payload(_)));
}

#[rstest]
#[case::internal_user("internal_user")]
#[case::proxy_admin_viewer("proxy_admin_viewer")]
#[tokio::test]
async fn a_role_other_than_proxy_admin_is_forbidden(#[case] role: &str) {
    let harness = harness(HarnessConfig::new());
    let route = session_route(&harness);
    let auth = auth_with(Some("user-1"), role);
    let Err(error) = add_session_server(&route, SESSION_BODY.as_bytes(), &auth).await else {
        panic!("only PROXY_ADMIN creates temporary sessions");
    };
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    let SessionError::Forbidden(detail) = error else {
        panic!("the refusal carries the fixed detail");
    };
    assert_eq!(
        detail.get("error").map(String::as_str),
        Some(FORBIDDEN_MESSAGE)
    );
}

#[tokio::test]
async fn a_caller_without_a_user_id_is_created_by_the_default_name() {
    let harness = harness(HarnessConfig::new());
    let route = session_route(&harness);
    let auth = auth_with(None, "proxy_admin");
    let Ok(record) = add_session_server(&route, SESSION_BODY.as_bytes(), &auth).await else {
        panic!("the admin creates the session");
    };
    assert_eq!(record.created_by.as_deref(), Some("default_user_id"));
}

#[tokio::test]
async fn a_draft_write_failure_answers_500_and_keeps_the_local_copy() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::DraftWrite));
    let database = harness.database.as_ref().expect("the database is set");
    database.put_server(row("test-session-1"));
    let route = session_route(&harness);
    let auth = auth_with(Some("admin-1"), "proxy_admin");
    let body =
        r#"{"server_id":"test-session-1","url":"https://mcp.example.test/sse","transport":"sse"}"#;
    let Err(error) = add_session_server(&route, body.as_bytes(), &auth).await else {
        panic!("the draft write fails");
    };
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.to_string(),
        "Error caching temporary mcp server: the inserted MCP server row is missing"
    );
    let cached = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the local lookup has no error")
        .expect("the local entry stays when the draft write fails");
    assert_eq!(cached.fields.server_id, "test-session-1");
}

#[tokio::test]
async fn a_database_error_before_the_caching_scope_is_not_wrapped() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::ServerRead));
    let route = session_route(&harness);
    let auth = auth_with(Some("admin-1"), "proxy_admin");
    let Err(error) = add_session_server(&route, SESSION_BODY.as_bytes(), &auth).await else {
        panic!("the id lookup fails");
    };
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(matches!(error, SessionError::Table(_)));
    assert!(
        !error
            .to_string()
            .starts_with("Error caching temporary mcp server:")
    );
}

#[tokio::test]
async fn a_body_that_is_not_json_answers_422() {
    let harness = harness(HarnessConfig::new());
    let route = session_route(&harness);
    let auth = auth_with(Some("admin-1"), "proxy_admin");
    let Err(error) = add_session_server(&route, b"{not json", &auth).await else {
        panic!("an undecodable body is rejected");
    };
    assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(matches!(error, SessionError::Body(_)));
}

#[test]
fn a_request_without_a_key_is_unauthenticated() {
    let outcome = authenticate::<AccessFailure>(None);
    assert!(matches!(outcome, Err(SessionError::Unauthenticated)));
}

#[test]
fn the_failure_facts_carry_the_status_and_the_message() {
    let error = SessionError::<AccessFailure>::Unauthenticated;
    let failure = management_failure(error);
    assert_eq!(failure.exception.class_name, "HTTPException");
    assert_eq!(
        failure.exception.message.as_deref(),
        Some("the caller is not authenticated")
    );
    assert_eq!(failure.exception.status_code, Some(401));
    assert!(failure.exception.class.is_none());
    assert!(failure.exception.code.is_none());
    assert!(failure.exception.response_status_code.is_none());
    assert!(failure.exception.llm_provider.is_none());
    assert!(failure.exception.category.is_none());
    assert!(failure.exception.rate_limit_type.is_none());
    assert!(failure.exception.budget_entity_type.is_none());
    assert!(failure.exception.budget_entity_id.is_none());
    assert!(failure.exception.budget_limit.is_none());
    assert!(failure.exception.budget_spend.is_none());
    assert!(failure.exception.proxy_type.is_none());
    assert!(failure.exception.headers.is_none());
    assert!(failure.exception.response_headers.is_none());
    assert!(failure.exception.litellm_response_headers.is_none());
    assert!(failure.exception.traceback_lines.is_empty());
    assert_eq!(failure.error.status(), StatusCode::UNAUTHORIZED);
}
