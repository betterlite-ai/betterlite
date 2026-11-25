#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::pass_through_registry::PassThroughRegistry;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{VirtualKeyRouteRule, VirtualKeyRouteStageInput};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";

const ALLOWED_KEY: &str = r#"{"token":"key-1","allowed_routes":["/v1/files"]}"#;
const DENIED_KEY: &str = r#"{"token":"key-1","allowed_routes":["/v1/embeddings"]}"#;

fn config_with_master_key() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .expect("the test config loads")
}

fn master_context(master_key: &SecretString) -> AuthContext {
    let input = StageInput::MasterKey(Some(master_key.clone()));
    let Ok(context) = dispatch(AuthBackend::MasterKey, &input, master_key) else {
        panic!("the master key stage authenticates");
    };
    context
}

fn key(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fields decode");
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs")
}

fn rule(fields: &str) -> Arc<VirtualKeyRouteRule> {
    Arc::new(VirtualKeyRouteRule {
        key: Some(key(fields)),
    })
}

fn stage_input(
    context: AuthContext,
    rule: Arc<VirtualKeyRouteRule>,
    route: &str,
) -> VirtualKeyRouteStageInput {
    VirtualKeyRouteStageInput {
        context,
        rule,
        team_allowed_routes: Vec::new(),
        registry: PassThroughRegistry {
            root_path: String::new(),
            registrations: Vec::new(),
        },
        route: route.to_owned(),
        method: Some("POST".to_owned()),
    }
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthVirtualKeyRoutes));
    let state = state_for(&config_with_master_key());
    assert_eq!(
        state.virtual_key_routes.team_allowed_routes,
        [
            "openai_routes",
            "info_routes",
            "mcp_routes",
            "/v1/messages",
            "/v1/messages/count_tokens",
        ]
    );
    let snapshot = state.virtual_key_routes.registry_snapshot();
    assert!(snapshot.registrations.is_empty());

    let master_key = SecretString::from("secret-master-key".to_owned());
    let granted = stage_input(master_context(&master_key), rule(ALLOWED_KEY), MODEL_ROUTE);
    let granted_input = StageInput::AuthVirtualKeyRoutes(Box::new(granted));
    let Ok(forwarded) = dispatch(
        AuthBackend::AuthVirtualKeyRoutes,
        &granted_input,
        &master_key,
    ) else {
        panic!("the B732 constructor dispatches to the production virtual key route stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let refused = stage_input(master_context(&master_key), rule(DENIED_KEY), MODEL_ROUTE);
    let refused_input = StageInput::AuthVirtualKeyRoutes(Box::new(refused));
    assert!(matches!(
        dispatch(
            AuthBackend::AuthVirtualKeyRoutes,
            &refused_input,
            &master_key
        ),
        Err(StageFailure::VirtualKeyRoutes(_))
    ));
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

fn request_to(reached: &Arc<AtomicBool>) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .body(Body::empty())
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(reached));
    request
}

fn application(config: &Config, rule: Arc<VirtualKeyRouteRule>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule))
}

async fn run_request(
    config: &Config,
    rule: Arc<VirtualKeyRouteRule>,
) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(config, rule)
        .oneshot(request_to(&reached))
        .await
        .expect("the request reaches the router");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the response body is UTF-8");
    (status, text, reached.load(Ordering::Acquire))
}

#[tokio::test]
async fn failure_stops_downstream() {
    let (status, body, reached) = run_request(&config_with_master_key(), rule(DENIED_KEY)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"403\""), "{body}");
    assert!(
        body.contains(
            "Virtual key is not allowed to call this route. Only allowed to call routes: \
             ['/v1/embeddings']. Tried to call route: /v1/files"
        ),
        "{body}"
    );
    assert!(!reached);

    let (allowed_status, _allowed_body, allowed_reached) =
        run_request(&config_with_master_key(), rule(ALLOWED_KEY)).await;
    assert_eq!(allowed_status, StatusCode::OK);
    assert!(allowed_reached);
}
