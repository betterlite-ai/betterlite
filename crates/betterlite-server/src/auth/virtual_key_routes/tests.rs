#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Extension, State};
use axum::http::{Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::post;
use betterlite_auth::auth_identity::records::SourceKey;
use betterlite_auth::pass_through_registry::{PassThroughMetadata, PassThroughRouteType};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{run_stage, stage_input};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::auth::failures::stage_failure_response;
use crate::error_body::AuthenticationFailure;
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

// the auth chain resolves the caller's key into the principal, and no other input carries it
fn context_with_key(master_key: &SecretString, fields: &str) -> AuthContext {
    let mut context = master_context(master_key);
    context.principal.source_key = SourceKey(Some(key(fields)));
    context
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

// the production stage constructor and runner behind a real HTTP route
async fn guard(State(context): State<AuthContext>, request: Request<Body>, next: Next) -> Response {
    let Ok(input) = stage_input(&request, Some(&context)) else {
        return stage_failure_response(&StageFailure::from(AuthenticationFailure::Missing));
    };
    let StageInput::AuthVirtualKeyRoutes(stage) = input else {
        panic!("the virtual key route constructor builds its own stage input");
    };
    match run_stage(&stage) {
        Ok(_) => next.run(request).await,
        Err(exception) => stage_failure_response(&StageFailure::VirtualKeyRoutes(exception)),
    }
}

fn guarded_application(config: &Config, context: AuthContext) -> Router {
    Router::new()
        .route(MODEL_ROUTE, post(mark_called))
        .layer(middleware::from_fn_with_state(context, guard))
        .layer(Extension(state_for(config)))
}

fn chained_application(config: &Config) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

async fn run_request(application: Router) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application
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

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthVirtualKeyRoutes));
    let config = config_with_master_key();
    let state = state_for(&config);
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

    let master_key = SecretString::from("secret-master-key".to_owned());
    let mut request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .body(Body::empty())
        .expect("the request builds");
    request.extensions_mut().insert(state);
    let Ok(granted) = stage_input(&request, Some(&context_with_key(&master_key, ALLOWED_KEY)))
    else {
        panic!("the production constructor builds the grant input");
    };
    let Ok(forwarded) = dispatch(AuthBackend::AuthVirtualKeyRoutes, &granted, &master_key) else {
        panic!("the B732 constructor dispatches to the production virtual key route stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let Ok(refused) = stage_input(&request, Some(&context_with_key(&master_key, DENIED_KEY)))
    else {
        panic!("the production constructor builds the denied input");
    };
    assert!(matches!(
        dispatch(AuthBackend::AuthVirtualKeyRoutes, &refused, &master_key),
        Err(StageFailure::VirtualKeyRoutes(_))
    ));
}

#[tokio::test]
async fn failure_stops_downstream() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let (status, body, reached) = run_request(guarded_application(
        &config_with_master_key(),
        context_with_key(&master_key, DENIED_KEY),
    ))
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"403\""), "{body}");
    assert!(body.contains("the key may not call /v1/files"), "{body}");
    assert!(
        body.contains("its allowed routes are /v1/embeddings"),
        "{body}"
    );
    assert!(!reached);

    let (allowed_status, _allowed_body, allowed_reached) = run_request(guarded_application(
        &config_with_master_key(),
        context_with_key(&master_key, ALLOWED_KEY),
    ))
    .await;
    assert_eq!(allowed_status, StatusCode::OK);
    assert!(allowed_reached);
}

#[test]
fn registered_routes_reach_the_stage() {
    let state = state_for(&config_with_master_key());
    state
        .virtual_key_routes
        .registry
        .lock()
        .unwrap()
        .registrations
        .push((
            "pass-a:exact:/team-pass:POST".to_owned(),
            PassThroughMetadata {
                endpoint_id: "pass-a".to_owned(),
                route_type: PassThroughRouteType::Exact,
                path: "/team-pass".to_owned(),
                methods: vec!["POST".to_owned()],
                auth: Some(true),
            },
        ));
    let mut request = request_to(&Arc::new(AtomicBool::new(false)));
    request.extensions_mut().insert(state);
    let master_key = SecretString::from("secret-master-key".to_owned());
    let context = master_context(&master_key);
    let Ok(StageInput::AuthVirtualKeyRoutes(input)) = stage_input(&request, Some(&context)) else {
        panic!("the virtual key route constructor builds its own stage input");
    };
    let registration = input
        .registry
        .get_registered("/team-pass", Some("POST"))
        .unwrap();
    assert_eq!(registration.auth, Some(true));
    assert_eq!(registration.endpoint_id, "pass-a");
}

#[tokio::test]
async fn a_request_without_a_key_has_no_route_restriction() {
    let (status, _body, reached) =
        run_request(chained_application(&config_with_master_key())).await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
}
