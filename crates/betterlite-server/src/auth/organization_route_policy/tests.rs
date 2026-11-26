#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::organization_route_policy::membership::{OrganizationMembership, RouteUser};
use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{OrganizationPolicyStageInput, OrganizationRouteRule};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

const TEAM_CREATE_ROUTE: &str = "/team/new";
const MASTER_KEY: &str = "secret-master-key";

fn route_user() -> RouteUser {
    RouteUser {
        user_role: None,
        organization_memberships: Some(vec![OrganizationMembership {
            organization_id: Some("org-a".to_owned()),
            user_role: Some("org_admin".to_owned()),
        }]),
    }
}

fn organization_body(organization_id: &str) -> JsonObject {
    let mut body = JsonObject::new();
    body.insert(
        "organization_id".to_owned(),
        Json::Text(organization_id.to_owned()),
    );
    body
}

fn master_context(master_key: &SecretString) -> AuthContext {
    let Ok(context) = dispatch(
        AuthBackend::MasterKey,
        &StageInput::MasterKey(Some(master_key.clone())),
        master_key,
    ) else {
        panic!("the master key stage authenticates");
    };
    context
}

fn stage_input(context: AuthContext, body: JsonObject) -> StageInput {
    StageInput::OrganizationRoutePolicy(Box::new(OrganizationPolicyStageInput {
        context,
        user: Some(route_user()),
        route: TEAM_CREATE_ROUTE.to_owned(),
        body,
    }))
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

// the real router with its auth chain, a handler that records whether it ran, and the rule the auth service would insert
async fn run_request(
    rule: OrganizationRouteRule,
    body: &'static str,
) -> (StatusCode, String, bool) {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .unwrap();
    let application = routes::router(
        state_for(&config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(TEAM_CREATE_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::new(rule)));
    let reached = Arc::new(AtomicBool::new(false));
    let mut request = Request::builder()
        .method("POST")
        .uri(TEAM_CREATE_ROUTE)
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
    let position = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::OrganizationRoutePolicy);
    let after_team_budgets = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::TeamBudgetAdmission)
        .map(|index| index.saturating_add(1));
    assert_eq!(position, after_team_budgets);
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let denied = stage_input(master_context(&master_key), JsonObject::new());
    let Err(StageFailure::OrganizationRoutePolicy(exception)) =
        dispatch(AuthBackend::OrganizationRoutePolicy, &denied, &master_key)
    else {
        panic!("the B732 constructor dispatches to the production organization policy stage");
    };
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param, Some("organization_id".to_owned()));
    assert_eq!(exception.code, "401");
    // an org admin that names its organization passes the same constructor
    let admitted = stage_input(master_context(&master_key), organization_body("org-a"));
    let Ok(forwarded) = dispatch(AuthBackend::OrganizationRoutePolicy, &admitted, &master_key)
    else {
        panic!("the organization policy admits the selected organization");
    };
    assert_eq!(forwarded.principal.subject, "master_key");
    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(
            AuthBackend::OrganizationRoutePolicy,
            &wrong_stage,
            &master_key
        ),
        Err(StageFailure::Authentication(_))
    ));
}

#[tokio::test]
async fn failure_stops_downstream() {
    let rule = OrganizationRouteRule {
        user: Some(route_user()),
    };
    let (status, body, reached) = run_request(rule, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"organization_id\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(
        body.contains("You are part of multiple organizations: org-a"),
        "{body}"
    );
    assert!(!reached);
}

#[tokio::test]
async fn admitted_request_reaches_downstream() {
    let rule = OrganizationRouteRule {
        user: Some(route_user()),
    };
    let (status, body, reached) = run_request(rule, "{\"organization_id\":\"org-a\"}").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "reached");
    assert!(reached);
}
