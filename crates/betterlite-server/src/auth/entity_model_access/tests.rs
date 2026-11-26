#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::organization::OrganizationTable;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{EntityModelRule, EntityModelStageInput};
use crate::auth::team_budget_admission::TeamBudgetRule;

mod members;
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::auth::prefetch_stage;
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";

const KEY: &str = r#"{"token":"hash-1","models":[]}"#;
const DENYING_ORG: &str = r#"{"organization_id":"org-1","budget_id":"budget-1","created_by":"creator","updated_by":"editor","models":["gpt-4o"]}"#;

fn config_with_master_key() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .expect("the test config loads")
}

fn config_with_database() -> Config {
    let mut lookup = BTreeMap::<String, String>::new();
    lookup.insert(
        "DATABASE_URL".to_owned(),
        "postgresql://user:password@localhost:5432/betterlite".to_owned(),
    );
    ConfigLoader { lookup }
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

fn key() -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(KEY).expect("the key fields decode");
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs")
}

fn org(row: &str) -> OrganizationTable {
    serde_json::from_str(row).expect("the organization fixture decodes")
}

fn rule(models: &str) -> Arc<EntityModelRule> {
    Arc::new(EntityModelRule {
        key: Some(key()),
        user: None,
        org: Some(org(models)),
        model: RequestedModels::One("claude".to_owned()),
        view: None,
        provider: None,
        global_model_aliases: BTreeMap::new(),
        teams: None,
        projects: None,
        member_checks: Arc::new(TeamBudgetRule::default()),
        now: || 0.0_f64,
    })
}

fn stage(master_key: &SecretString, models: &str) -> EntityModelStageInput {
    EntityModelStageInput {
        context: master_context(master_key),
        rule: Some(rule(models)),
        max_recurse_depth: 100,
    }
}

#[test]
fn rule_debug_names_its_fields() {
    // missing_debug_implementations requires the impl, so a formatted rule has to name what it holds
    let printed = format!("{:?}", rule(DENYING_ORG));
    assert!(printed.contains("has_user: false"), "{printed}");
}

#[tokio::test]
async fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::EntityModelAccess));
    // LiteLLM reads teams, access groups and projects only when a database is configured
    let without = state_for(&config_with_master_key());
    assert!(!without.entity_model_access.database);
    let with = state_for(&config_with_database());
    assert!(with.entity_model_access.database);

    let master_key = SecretString::from("secret-master-key".to_owned());
    let granted = stage(
        &master_key,
        r#"{"budget_id":"b","created_by":"a","updated_by":"b","models":["claude"]}"#,
    );
    let granted_input = StageInput::EntityModelAccess(Box::new(granted));
    let Ok(forwarded) = dispatch(AuthBackend::EntityModelAccess, &granted_input, &master_key)
    else {
        panic!("the B732 constructor dispatches to the production entity model access stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let refused = stage(&master_key, DENYING_ORG);
    let refused_input = StageInput::EntityModelAccess(Box::new(refused));
    assert!(matches!(
        prefetch_stage(refused_input).await,
        Err(StageFailure::ModelAccess(_))
    ));

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::EntityModelAccess, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
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

fn application(config: &Config, rule: Arc<EntityModelRule>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule))
}

async fn run_request(config: &Config, rule: Arc<EntityModelRule>) -> (StatusCode, String, bool) {
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
    let (status, body, reached) = run_request(&config_with_master_key(), rule(DENYING_ORG)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body.contains("\"type\":\"org_model_access_denied\""),
        "{body}"
    );
    assert!(body.contains("\"param\":\"model\""), "{body}");
    assert!(body.contains("\"code\":\"403\""), "{body}");
    assert!(
        body.contains("The requested model 'claude' is not available for this API key"),
        "{body}"
    );
    assert!(!reached);

    let (allowed_status, _allowed_body, allowed_reached) = run_request(
        &config_with_master_key(),
        rule(r#"{"budget_id":"b","created_by":"a","updated_by":"b","models":["claude"]}"#),
    )
    .await;
    assert_eq!(allowed_status, StatusCode::OK);
    assert!(allowed_reached);
}

#[tokio::test]
async fn live_member_scope_stops_downstream() {
    let (grants, reads) = members::rule_with_member(true);
    let (status, body, reached) = run_request(&config_with_master_key(), grants).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("team_model_access_denied"), "{body}");
    assert!(!reached);
    assert!(reads.load(Ordering::Acquire));
}

#[tokio::test]
async fn fallback_team_skips_member_scope() {
    let (grants, reads) = members::rule_with_member(false);
    let (status, body, reached) = run_request(&config_with_master_key(), grants).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(reached);
    assert!(!reads.load(Ordering::Acquire));
}
