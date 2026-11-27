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
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{AuthPolicySupportRule, AuthPolicySupportStageInput};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";
const JSON: Option<&str> = Some("application/json");
const MODIFICATION_BODY: &str = r#"{"metadata":{"guardrails":[]}}"#;
const CHAT_BODY: &str = r#"{"tools":[{"type":"function","function":{"name":"weather"}}]}"#;
const DENYING_TEAM: &str =
    r#"{"team_id":"team-1","metadata":{"guardrails":{"modify_guardrails":false}}}"#;

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

fn team(fields: &str) -> LiteLLMTeamTable {
    serde_json::from_str(fields).expect("the team fixture decodes")
}

fn key(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fields decode");
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs")
}

fn stage(
    rule: AuthPolicySupportRule,
    route: &str,
    body: &'static str,
) -> AuthPolicySupportStageInput {
    let state = state_for(&config_with_master_key());
    let master_key = SecretString::from("secret-master-key".to_owned());
    AuthPolicySupportStageInput {
        context: master_context(&master_key),
        rule: Arc::new(rule),
        route: route.to_owned(),
        content_type: JSON.map(str::to_owned),
        body: Some(Bytes::from_static(body.as_bytes())),
        slot: Some(state.auth_policy_support.clone()),
    }
}

fn denying_rule() -> AuthPolicySupportRule {
    AuthPolicySupportRule {
        key: None,
        team: Some(team(DENYING_TEAM)),
    }
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthPolicySupport));

    let master_key = SecretString::from("secret-master-key".to_owned());
    let allowed = stage(
        AuthPolicySupportRule::default(),
        MODEL_ROUTE,
        MODIFICATION_BODY,
    );
    let allowed_input = StageInput::AuthPolicySupport(Box::new(allowed));
    let Ok(forwarded) = dispatch(AuthBackend::AuthPolicySupport, &allowed_input, &master_key)
    else {
        panic!("the B732 constructor dispatches to the production policy stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let refused_input = StageInput::AuthPolicySupport(Box::new(stage(
        denying_rule(),
        MODEL_ROUTE,
        MODIFICATION_BODY,
    )));
    assert!(matches!(
        dispatch(AuthBackend::AuthPolicySupport, &refused_input, &master_key),
        Err(StageFailure::AuthPolicySupport(_))
    ));

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthPolicySupport, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));
}

// the pinned B207 catalog must name the chat route tool-capable for the allowlist to see its tools
#[test]
fn tool_allowlist_reads_the_pinned_catalog() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let restricted = AuthPolicySupportRule {
        key: Some(key(
            r#"{"token":"key-1","metadata":{"allowed_tools":["other"]}}"#,
        )),
        team: None,
    };
    let denied_input = StageInput::AuthPolicySupport(Box::new(stage(
        restricted,
        "/v1/chat/completions",
        CHAT_BODY,
    )));
    assert!(matches!(
        dispatch(AuthBackend::AuthPolicySupport, &denied_input, &master_key),
        Err(StageFailure::AuthPolicySupport(_))
    ));

    let granted = AuthPolicySupportRule {
        key: Some(key(
            r#"{"token":"key-2","metadata":{"allowed_tools":["weather"]}}"#,
        )),
        team: None,
    };
    let granted_input =
        StageInput::AuthPolicySupport(Box::new(stage(granted, "/v1/chat/completions", CHAT_BODY)));
    let Ok(granted_context) = dispatch(AuthBackend::AuthPolicySupport, &granted_input, &master_key)
    else {
        panic!("an allowlisted tool name passes the policy stage");
    };
    assert_eq!(granted_context.principal.subject, "master_key");
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
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(MODIFICATION_BODY))
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(reached));
    request
}

fn application(rule: AuthPolicySupportRule) -> Router {
    let state = state_for(&config_with_master_key());
    routes::router(
        state,
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::new(rule)))
}

async fn run_request(rule: AuthPolicySupportRule) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(rule)
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
    let (status, body, reached) = run_request(denying_rule()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"403\""), "{body}");
    assert!(
        body.contains("Your team does not have permission to modify guardrails."),
        "{body}"
    );
    assert!(!reached);
}

#[tokio::test]
async fn permitted_requests_reach_the_handler() {
    let (status, _body, reached) = run_request(AuthPolicySupportRule::default()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
}
