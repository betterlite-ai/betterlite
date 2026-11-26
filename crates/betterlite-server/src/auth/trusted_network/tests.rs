#![cfg(test)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{ConnectInfo, Extension};
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::routing::post;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use secrecy::SecretString;
use tower::ServiceExt as _;

use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageInput, dispatch,
};
use crate::auth::{prefetch_stage, resolver_for, stage_input};
use crate::routes;
use crate::routes::tests::state_for;

const MODEL_ROUTE: &str = "/v1/files";
const MASTER_KEY: &str = "secret-master-key";
const PROXY_PEER: &str = "10.0.0.5";
const CLIENT_IP: &str = "203.0.113.7";
const OVERSIZE_HEADER: usize = 0x0010_0001;

fn config_with(settings: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!(
        "general_settings:\n  master_key: {MASTER_KEY}\n{settings}"
    ))
    .expect("the test config loads")
}

fn proxy_config() -> Config {
    config_with("  trusted_proxy_ranges: [10.0.0.0/8]\n")
}

fn master_context() -> AuthContext {
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let input = StageInput::MasterKey(Some(master_key.clone()));
    let Ok(context) = dispatch(AuthBackend::MasterKey, &input, &master_key) else {
        panic!("the master key stage authenticates");
    };
    context
}

fn request(config: &Config) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, format!("Bearer {MASTER_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", CLIENT_IP)
        .header(header::HOST, "proxy.example")
        .extension(ConnectInfo(SocketAddr::from(([10, 0, 0, 5], 4000))))
        .extension(state_for(config))
        .body(Body::from(r#"{"model":"gpt-4o","messages":[]}"#))
        .expect("the request builds")
}

#[test]
fn stage_is_registered() {
    let position = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::TrustedNetwork)
        .expect("the trusted network stage is installed");
    let master = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::MasterKey)
        .expect("the master key stage is installed");
    assert_eq!(
        position,
        master.saturating_add(1),
        "network identity is stamped right after authentication"
    );

    let config = proxy_config();
    let authenticated = master_context();
    let incoming = request(&config);
    let resolver = resolver_for(&incoming);
    let Ok(staged) = stage_input(
        AuthBackend::TrustedNetwork,
        &incoming,
        Some(&authenticated),
        &resolver,
        None,
    ) else {
        panic!("the production constructor builds the network stage input");
    };
    let StageInput::TrustedNetwork(stage) = &staged else {
        panic!("the constructor returns the network stage input");
    };
    assert_eq!(stage.peer.as_deref(), Some(PROXY_PEER));
    assert_eq!(stage.document.x_forwarded_for.as_deref(), Some(CLIENT_IP));
}

#[tokio::test]
async fn stage_resolves_the_client_network() {
    let config = proxy_config();
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let authenticated = master_context();
    let incoming = request(&config);
    let resolver = resolver_for(&incoming);
    let Ok(staged) = stage_input(
        AuthBackend::TrustedNetwork,
        &incoming,
        Some(&authenticated),
        &resolver,
        None,
    ) else {
        panic!("the production constructor builds the network stage input");
    };
    let Ok(prefetched) = prefetch_stage(staged).await else {
        panic!("the network stage has no prefetch to fail");
    };
    let Ok(context) = dispatch(AuthBackend::TrustedNetwork, &prefetched, &master_key) else {
        panic!("a request from a configured trusted proxy resolves");
    };
    assert_eq!(
        context.principal.network.client_ip.as_deref(),
        Some(CLIENT_IP)
    );
    assert_eq!(
        context.principal.network.host.as_deref(),
        Some("proxy.example")
    );
    assert!(context.principal.network.via_trusted_proxy);
}

#[test]
fn header_bytes_decode_as_latin1() {
    let config = proxy_config();
    let authenticated = master_context();
    let incoming = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .extension(state_for(&config))
        .header(
            "x-forwarded-for",
            HeaderValue::from_bytes(b"\x80\x81proxy").expect("an obs-text header value is allowed"),
        )
        .body(Body::empty())
        .expect("the request builds");
    let resolver = resolver_for(&incoming);
    let Ok(staged) = stage_input(
        AuthBackend::TrustedNetwork,
        &incoming,
        Some(&authenticated),
        &resolver,
        None,
    ) else {
        panic!("the production constructor builds the network stage input");
    };
    let StageInput::TrustedNetwork(stage) = &staged else {
        panic!("the constructor returns the network stage input");
    };
    // no ConnectInfo extension: the request has no direct peer
    assert_eq!(stage.peer, None);
    assert_eq!(
        stage.document.x_forwarded_for.as_deref(),
        Some("\u{80}\u{81}proxy")
    );
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>, body: Bytes) -> String {
    reached.store(true, Ordering::Release);
    String::from_utf8(body.to_vec()).unwrap_or_default()
}

fn application(config: &Config, reached: &Arc<AtomicBool>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::clone(reached)))
}

fn oversize_request() -> Request<Body> {
    let mut incoming = request(&proxy_config());
    let oversized = HeaderValue::from_bytes("a".repeat(OVERSIZE_HEADER).as_bytes())
        .expect("the oversize header builds");
    incoming.headers_mut().insert("x-forwarded-for", oversized);
    incoming
}

#[tokio::test]
async fn failure_stops_downstream() {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(&proxy_config(), &reached)
        .oneshot(oversize_request())
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(bytes.to_vec()).expect("the response body is UTF-8");
    assert!(text.contains("\"type\":\"auth_error\""), "{text}");
    assert!(text.contains("\"param\":\"None\""), "{text}");
    assert!(text.contains("\"code\":\"401\""), "{text}");
    assert!(text.contains("Authentication Error"), "{text}");
    assert!(!reached.load(Ordering::Acquire));
}

#[test]
fn missing_general_settings_fall_back_to_defaults() {
    let authenticated = master_context();
    let incoming = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header("x-forwarded-for", CLIENT_IP)
        .body(Body::empty())
        .expect("the request builds");
    let resolver = resolver_for(&incoming);
    let Ok(staged) = stage_input(
        AuthBackend::TrustedNetwork,
        &incoming,
        Some(&authenticated),
        &resolver,
        None,
    ) else {
        panic!("the production constructor builds the network stage input without a loaded config");
    };
    let StageInput::TrustedNetwork(stage) = &staged else {
        panic!("the constructor returns the network stage input");
    };
    assert_eq!(stage.document.trusted_proxy_ranges, None);
    assert_eq!(stage.document.mcp_trusted_proxy_ranges, None);
    assert_eq!(stage.document.x_forwarded_for.as_deref(), Some(CLIENT_IP));
}
