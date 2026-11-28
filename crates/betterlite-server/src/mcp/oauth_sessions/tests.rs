#![cfg(test)]

pub(crate) mod testing;

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_mcp::registry::resolution::{AuthorizeError, ResolveError};
use betterlite_mcp::server_build::BuildError;
use betterlite_mcp::server_payload::{PayloadError, RequestError};
use serde::Deserialize;

use super::SessionError;
use crate::mcp::admin_credential_view::CredentialViewError;
use crate::mcp::oauth_sessions::record::RecordError;
use crate::mcp::server_writes::ServerWriteError;

#[derive(Debug)]
struct AccessGone;

impl Display for AccessGone {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("the access check is unavailable")
    }
}

impl StdError for AccessGone {}

fn shape_error() -> serde_json::Error {
    serde_json::from_str::<u8>("not a number").expect_err("the fixture is not a number")
}

fn detail(message: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("error".to_owned(), message.to_owned())])
}

#[derive(Deserialize)]
struct RenderedDetail {
    detail: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct AuthErrorBody {
    error: AuthErrorDetail,
}

#[derive(Deserialize)]
struct AuthErrorDetail {
    message: String,
    code: String,
}

async fn body_of(response: Response) -> Vec<u8> {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body reads");
    body.to_vec()
}

#[test]
fn request_and_body_failures_map_to_their_statuses() {
    let body = SessionError::<AccessGone>::Body(shape_error());
    assert_eq!(body.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let value = SessionError::<AccessGone>::Request(RequestError::Value(
        "url or spec_path is required for HTTP/SSE transport".to_owned(),
    ));
    assert_eq!(value.status(), StatusCode::BAD_REQUEST);
    let shape = SessionError::<AccessGone>::Request(RequestError::Shape(shape_error()));
    assert_eq!(shape.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[test]
fn payload_failures_answer_400_except_an_unknown_required_field() {
    let separator = PayloadError::SeparatorInName {
        separator: "-".to_owned(),
        name: "bad-name".to_owned(),
    };
    assert_eq!(
        SessionError::<AccessGone>::Payload(separator).status(),
        StatusCode::BAD_REQUEST
    );
    let unknown = PayloadError::UnknownRequiredFields {
        unknown: "'nope'".to_owned(),
    };
    assert_eq!(
        SessionError::<AccessGone>::Payload(unknown).status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[test]
fn lookup_denials_and_internal_failures_map_to_their_statuses() {
    let missing =
        SessionError::<AccessGone>::NotFound(detail("MCP server test-session-1 not found"));
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let denied =
        SessionError::<AccessGone>::Forbidden(detail("Access denied to MCP server test-session-1"));
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let unauthenticated = SessionError::<AccessGone>::Unauthenticated;
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    let no_runtime = SessionError::<AccessGone>::MissingRuntime {
        server_id: "test-session-1".to_owned(),
    };
    assert_eq!(no_runtime.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn upstream_error_variants_all_answer_500() {
    let access = SessionError::<AccessGone>::Access(AccessGone);
    assert_eq!(access.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let table = SessionError::<AccessGone>::Table(McpTableError::MissingServerId);
    assert_eq!(table.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let write = SessionError::<AccessGone>::Write(ServerWriteError::InsertedRowMissing);
    assert_eq!(write.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let build = SessionError::<AccessGone>::Build(BuildError::PrefixExhausted {
        server_id: "test-session-1".to_owned(),
        attempts: 3,
    });
    assert_eq!(build.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let resolve = SessionError::<AccessGone>::Resolve(ResolveError::Table(Box::new(
        McpTableError::MissingServerId,
    )));
    assert_eq!(resolve.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let view = SessionError::<AccessGone>::CredentialView(CredentialViewError::Json(shape_error()));
    assert_eq!(view.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let record = SessionError::<AccessGone>::Record(RecordError::Timestamp {
        value: "not a timestamp".to_owned(),
        source: shape_error(),
    });
    assert_eq!(record.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn a_caching_failure_names_the_step_in_its_text() {
    let inner = SessionError::<AccessGone>::Write(ServerWriteError::InsertedRowMissing);
    let error = SessionError::<AccessGone>::Caching {
        source: Box::new(inner),
    };
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.to_string(),
        "Error caching temporary mcp server: the inserted MCP server row is missing"
    );
}

#[test]
fn authorize_errors_keep_their_kind() {
    let missing = SessionError::<AccessGone>::from(AuthorizeError::NotFound(detail("gone")));
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let denied = SessionError::<AccessGone>::from(AuthorizeError::Forbidden(detail("no")));
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let failed = SessionError::<AccessGone>::from(AuthorizeError::<AccessGone>::Access(AccessGone));
    assert!(matches!(failed, SessionError::Access(_)));
    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn a_not_found_response_carries_the_detail_map() {
    let error = SessionError::<AccessGone>::NotFound(detail("MCP server test-session-1 not found"));
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = body_of(response).await;
    let rendered: RenderedDetail =
        serde_json::from_slice(&body).expect("the body is the detail map");
    assert_eq!(
        rendered.detail.get("error"),
        Some(&"MCP server test-session-1 not found".to_owned())
    );
}

#[tokio::test]
async fn a_forbidden_response_carries_the_detail_map() {
    let error =
        SessionError::<AccessGone>::Forbidden(detail("Access denied to MCP server test-session-1"));
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = body_of(response).await;
    let rendered: RenderedDetail =
        serde_json::from_slice(&body).expect("the body is the detail map");
    assert_eq!(
        rendered.detail.get("error"),
        Some(&"Access denied to MCP server test-session-1".to_owned())
    );
}

#[tokio::test]
async fn an_unauthenticated_response_is_the_auth_error_body() {
    let response = SessionError::<AccessGone>::Unauthenticated.into_response();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = body_of(response).await;
    let rendered: AuthErrorBody =
        serde_json::from_slice(&body).expect("the body is the auth error");
    assert_eq!(rendered.error.message, "the API key is not valid");
    assert_eq!(rendered.error.code, "401");
}

#[tokio::test]
async fn a_caching_response_renders_the_wrapped_message() {
    let inner = SessionError::<AccessGone>::Access(AccessGone);
    let error = SessionError::<AccessGone>::Caching {
        source: Box::new(inner),
    };
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_of(response).await;
    let rendered: RenderedDetail =
        serde_json::from_slice(&body).expect("the body is the detail map");
    assert_eq!(
        rendered.detail.get("error"),
        Some(&"Error caching temporary mcp server: the access check is unavailable".to_owned())
    );
}
