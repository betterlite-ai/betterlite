#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::user_repository::projection::user_record_to_model;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{AuthRoutePolicyRule, RoutePolicyStageInput, stage_input};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

const ROUTE: &str = "/user/new";
const CHAT_ROUTE: &str = "/v1/chat/completions";
const MASTER_KEY: &str = "secret-master-key";

fn loaded(admin_only_routes: &[&str]) -> Config {
    let mut source = format!("general_settings:\n  master_key: {MASTER_KEY}\n");
    if !admin_only_routes.is_empty() {
        let quoted: Vec<String> = admin_only_routes
            .iter()
            .map(|route| format!("'{route}'"))
            .collect();
        source.push_str("  admin_only_routes: [");
        source.push_str(&quoted.join(", "));
        source.push_str("]\n");
    }
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source)
    .unwrap()
}

fn master_key() -> SecretString {
    SecretString::from(MASTER_KEY.to_owned())
}

fn master_context() -> AuthContext {
    let key = master_key();
    let Ok(context) = dispatch(
        AuthBackend::MasterKey,
        &StageInput::MasterKey(Some(key.clone())),
        &key,
    ) else {
        panic!("the master key stage authenticates");
    };
    context
}

fn token() -> UserAPIKeyAuth {
    let record: BTreeMap<String, JsonData> =
        serde_json::from_str(r#"{"user_id":"user-1"}"#).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(record)).unwrap()
}

fn user_row() -> LiteLLMUserTable {
    let record: JsonData =
        serde_json::from_str(r#"{"user_id":"user-1","user_role":"internal_user"}"#).unwrap();
    user_record_to_model(Some(&record)).unwrap().unwrap()
}

fn rule() -> AuthRoutePolicyRule {
    AuthRoutePolicyRule {
        token: Some(token()),
        user: Some(user_row()),
    }
}

fn stage_from(
    rule: AuthRoutePolicyRule,
    uri: &str,
    method: &str,
    body: &'static str,
) -> RoutePolicyStageInput {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    request.extensions_mut().insert(Arc::new(rule));
    let context = master_context();
    let bytes = Bytes::from_static(body.as_bytes());
    let Ok(StageInput::AuthRoutePolicy(stage)) =
        stage_input(&request, Some(&context), Some(&bytes))
    else {
        panic!("the route policy builds its own stage input");
    };
    *stage
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

// the real router with its auth chain, a handler that records whether it ran, and the rule the auth service inserts
async fn run_request(
    config: &Config,
    rule: AuthRoutePolicyRule,
    route: &str,
    body: &'static str,
) -> (StatusCode, String, bool) {
    let application = routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(route, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::new(rule)));
    let reached = Arc::new(AtomicBool::new(false));
    let mut request = Request::builder()
        .method("POST")
        .uri(route)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    request.extensions_mut().insert(Arc::clone(&reached));
    let response = application.oneshot(request).await.unwrap();
    let status = response.status();
    let answer = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        String::from_utf8(answer.to_vec()).unwrap(),
        reached.load(Ordering::Acquire),
    )
}

#[test]
fn stage_is_registered() {
    // LiteLLM runs the route check after the model, budget and organization checks
    let position = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::AuthRoutePolicy);
    let after_grants = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::ModelGrantIntersection)
        .map(|index| index.saturating_add(1));
    assert_eq!(position, after_grants);
    let master = master_key();
    let denied = stage_from(rule(), ROUTE, "POST", "");
    let Err(StageFailure::AuthRoutePolicy(exception)) = dispatch(
        AuthBackend::AuthRoutePolicy,
        &StageInput::AuthRoutePolicy(Box::new(denied)),
        &master,
    ) else {
        panic!("the route policy denies a non-admin management route");
    };
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param, Some("None".to_owned()));
    assert_eq!(exception.code, "401");
    let wrong_stage = StageInput::MasterKey(Some(master.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthRoutePolicy, &wrong_stage, &master),
        Err(StageFailure::Authentication(_))
    ));
}

#[test]
fn stage_input_reads_method_and_query() {
    let info = stage_from(
        rule(),
        "/user/info?user_id=user-2&user_id=user-3",
        "GET",
        "",
    );
    assert_eq!(info.route, "/user/info");
    assert_eq!(info.method.as_deref(), Some("GET"));
    assert_eq!(info.user_id_query.as_deref(), Some("user-3"));
    // an info route never parses the body
    assert!(info.body.is_empty());
    let team = stage_from(rule(), "/team/update", "POST", r#"{"team_id":"team-1"}"#);
    assert_eq!(team.method.as_deref(), Some("POST"));
    assert!(team.user_id_query.is_none());
    assert!(!team.body.is_empty());
    // neither an inference route nor an info route pays for parsing the body
    let chat = stage_from(rule(), CHAT_ROUTE, "POST", r#"{"model":"gpt-4o"}"#);
    assert!(chat.body.is_empty());
    let info_with_body = stage_from(rule(), "/user/info", "POST", r#"{"user_id":"user-2"}"#);
    assert!(info_with_body.body.is_empty());
}

#[test]
fn master_key_request_is_inert() {
    let stage = stage_from(AuthRoutePolicyRule::default(), ROUTE, "POST", "");
    let Ok(context) = super::run_stage(&stage) else {
        panic!("the master key runs no route policy");
    };
    assert_eq!(context.principal.subject, "master_key");
}

#[tokio::test]
async fn failure_stops_downstream() {
    let configuration = loaded(&[]);
    let (status, body, reached) = run_request(&configuration, rule(), ROUTE, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(!reached);
}

#[tokio::test]
async fn master_key_request_reaches_downstream() {
    let configuration = loaded(&[]);
    let (status, body, reached) =
        run_request(&configuration, AuthRoutePolicyRule::default(), ROUTE, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "reached");
    assert!(reached);
}

#[tokio::test]
async fn configured_admin_only_route_stops_downstream() {
    let configuration = loaded(&[CHAT_ROUTE]);
    let (status, body, reached) = run_request(&configuration, rule(), CHAT_ROUTE, "{}").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.contains("general_settings.admin_only_routes"),
        "{body}"
    );
    assert!(!reached);
}
