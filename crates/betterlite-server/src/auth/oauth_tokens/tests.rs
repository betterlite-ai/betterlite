#![cfg(test)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Extension};
use axum::http::{Request, StatusCode};
use axum::routing::post;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use tower::ServiceExt as _;

use super::{PreStage, pre_stage};
use crate::auth::auth_identity::AuthContext;
use crate::routes;
use crate::routes::tests::state_for;

const CHAT_ROUTE: &str = "/v1/chat/completions";

fn proxy_config() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(
        "\
general_settings:
  master_key: secret-master-key
  enable_oauth2_proxy_auth: true
  trusted_proxy_ranges: [10.0.0.0/8]
  oauth2_config_mappings:
    user_id: X-User-Id
",
    )
    .expect("the test config loads")
}

fn introspection_config(endpoint: &str) -> Config {
    let lookup = BTreeMap::from([
        ("OAUTH_CLIENT_ID".to_owned(), "betterlite-client".to_owned()),
        (
            "OAUTH_CLIENT_SECRET".to_owned(),
            "betterlite-secret".to_owned(),
        ),
        ("OAUTH_TOKEN_INFO_ENDPOINT".to_owned(), endpoint.to_owned()),
    ]);
    ConfigLoader { lookup }
        .load(
            "\
general_settings:
  master_key: secret-master-key
  enable_oauth2_auth: true
",
        )
        .expect("the introspection config loads")
}

// the real B732 router constructor, with this production stage inside its auth middleware
fn application(config: &Config) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(CHAT_ROUTE, post(downstream)),
        Router::new(),
    )
}

async fn downstream(
    Extension(reached): Extension<Arc<AtomicBool>>,
    Extension(identity): Extension<AuthContext>,
) -> String {
    reached.store(true, Ordering::Release);
    identity.principal.subject
}

fn request_from(
    peer: SocketAddr,
    headers: &[(&str, &str)],
    reached: &Arc<AtomicBool>,
) -> Request<Body> {
    let mut builder = Request::builder().method("POST").uri(CHAT_ROUTE);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(Body::empty()).expect("the request builds");
    request.extensions_mut().insert(ConnectInfo(peer));
    request.extensions_mut().insert(Arc::clone(reached));
    request
}

fn peer(address: [u8; 4]) -> SocketAddr {
    SocketAddr::from((address, 4000))
}

#[tokio::test]
async fn stage_is_registered() {
    let config = proxy_config();
    let state = state_for(&config);
    assert!(
        state.oauth_tokens.settings.enable_oauth2_proxy_auth,
        "the production state reads the oauth setting"
    );
    let reached = Arc::new(AtomicBool::new(false));
    let request = request_from(
        peer([10, 0, 0, 5]),
        &[("x-user-id", "oauth-user")],
        &reached,
    );
    let response = application(&config)
        .oneshot(request)
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    assert_eq!(body.as_ref(), b"oauth-user");
    assert!(reached.load(Ordering::Acquire));
}

#[tokio::test]
async fn introspection_failure_stops_downstream() {
    let config = introspection_config("not-a-url");
    let reached = Arc::new(AtomicBool::new(false));
    let request = request_from(
        peer([127, 0, 0, 1]),
        &[("authorization", "Bearer opaque-token")],
        &reached,
    );
    let response = application(&config)
        .oneshot(request)
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the response body is UTF-8");
    assert!(text.contains("\"type\":\"auth_error\""), "{text}");
    assert!(
        text.contains("the OAuth token info endpoint is not a URL"),
        "{text}"
    );
    assert!(!reached.load(Ordering::Acquire));
}

#[tokio::test]
async fn route_kind_selects_the_introspection_branch() {
    let config = introspection_config("not-a-url");
    let state = state_for(&config);
    let mut info = Request::builder()
        .method("GET")
        .uri("/team/list")
        .header("authorization", "Bearer opaque-token")
        .body(Body::empty())
        .expect("the request builds");
    info.extensions_mut().insert(Arc::clone(&state));
    let info_stage = pre_stage(info.headers(), info.uri().path(), info.extensions()).await;
    assert!(matches!(info_stage, PreStage::Rejected(_)));
    let mut other = Request::builder()
        .method("GET")
        .uri("/team/delete")
        .header("authorization", "Bearer opaque-token")
        .body(Body::empty())
        .expect("the request builds");
    other.extensions_mut().insert(Arc::clone(&state));
    let other_stage = pre_stage(other.headers(), other.uri().path(), other.extensions()).await;
    assert!(matches!(other_stage, PreStage::Skipped));
}

#[tokio::test]
async fn missing_token_rejected_before_introspection() {
    let config = introspection_config("not-a-url");
    let reached = Arc::new(AtomicBool::new(false));
    let request = request_from(peer([127, 0, 0, 1]), &[], &reached);
    let response = application(&config)
        .oneshot(request)
        .await
        .expect("the request reaches the router");
    // the gateway's missing-key answer, not the introspection error the invalid endpoint would raise
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    assert_eq!(body.as_ref(), b"Internal Server Error");
    assert!(!reached.load(Ordering::Acquire));
}

#[tokio::test]
async fn failure_stops_downstream() {
    let config = proxy_config();
    let reached = Arc::new(AtomicBool::new(false));
    let request = request_from(
        peer([192, 0, 2, 9]),
        &[("x-user-id", "oauth-user")],
        &reached,
    );
    let response = application(&config)
        .oneshot(request)
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the response body is UTF-8");
    assert!(text.contains("\"type\":\"auth_error\""), "{text}");
    assert!(text.contains("\"param\":\"None\""), "{text}");
    assert!(text.contains("\"code\":\"401\""), "{text}");
    assert!(text.contains("is not trusted"), "{text}");
    assert!(!reached.load(Ordering::Acquire));
}
