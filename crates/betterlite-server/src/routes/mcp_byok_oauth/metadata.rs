mod tests;

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_mcp::byok_oauth_metadata::{
    authorization_server_document, prefixed_base_url, protected_resource_document,
};
use betterlite_mcp::byok_token_exchange::ports::{AuthCodeLookup, ByokCredentialSink};

use crate::error_body::json_error;
use crate::routes::mcp_byok_oauth::{ByokOauthRoutes, RequestBaseUrl};

#[derive(Clone)]
pub(crate) enum DocumentKind {
    Bare,
    Issuer,
    Prefixed(String),
    ProtectedResource,
}

pub(crate) async fn discovery_document<C, S, B>(
    State(state): State<Arc<ByokOauthRoutes<C, S, B>>>,
    Extension(kind): Extension<DocumentKind>,
    request: Request,
) -> Response
where
    C: AuthCodeLookup + 'static,
    S: ByokCredentialSink + 'static,
    B: RequestBaseUrl + 'static,
{
    let (parts, _body) = request.into_parts();
    let request_base = state.base_url.base_url(&parts);
    match kind {
        DocumentKind::Bare => {
            let document = authorization_server_document(&request_base, &request_base);
            Json(document).into_response()
        }
        DocumentKind::Issuer => {
            let issuer = format!("{request_base}/v1/mcp/oauth");
            Json(authorization_server_document(&request_base, &issuer)).into_response()
        }
        DocumentKind::Prefixed(prefix) => match prefixed_base_url(&request_base, &prefix) {
            Ok(base_url) => {
                let issuer = format!("{base_url}/v1/mcp/oauth");
                Json(authorization_server_document(&base_url, &issuer)).into_response()
            }
            Err(_error) => internal_server_error(),
        },
        DocumentKind::ProtectedResource => match protected_resource_document(&request_base) {
            Ok(document) => Json(document).into_response(),
            Err(_error) => internal_server_error(),
        },
    }
}

// LiteLLM's urlparse raises here and Starlette answers 500; the body follows betterlite's error envelope.
fn internal_server_error() -> Response {
    json_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal Server Error",
        "internal_server_error",
        "500",
    )
}
