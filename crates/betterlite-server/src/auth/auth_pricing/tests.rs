#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::current_spend::BUDGET_UNVERIFIABLE_DETAIL;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::auth_pricing::{MaxBudget, Settings};
use betterlite_config::types::Config;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{PricingRule, PricingStageInput};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized LLM route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";
// the request-level checks the task pins run on the chat route
const CHAT_ROUTE: &str = "/v1/chat/completions";
const MASTER_KEY: &str = "secret-master-key";

fn config_with(general: &str, max_budget: &str) -> Config {
    let source = format!(
        "general_settings:\n  master_key: {MASTER_KEY}\n{general}litellm_settings:\n  max_budget: {max_budget}\n"
    );
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source)
    .expect("the test config loads")
}

fn config(max_budget: &str) -> Config {
    config_with("", max_budget)
}

fn master_context(master_key: &SecretString) -> AuthContext {
    let input = StageInput::MasterKey(Some(master_key.clone()));
    let Ok(context) = dispatch(AuthBackend::MasterKey, &input, master_key) else {
        panic!("the master key stage authenticates");
    };
    context
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthPricing));
    let pricing = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::AuthPricing);
    let after_grants = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::AuthResourceGrants)
        .map(|index| index.saturating_add(1));
    assert_eq!(pricing, after_grants);

    // the config section reaches the state slot the stage reads
    let state = state_for(&config("5"));
    assert_eq!(
        state.auth_pricing.settings.max_budget,
        MaxBudget::Finite(Decimal::from(5_i64))
    );

    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let stage = PricingStageInput {
        context: master_context(&master_key),
        rule: Arc::new(PricingRule::default()),
        method: Some("POST".to_owned()),
        route: MODEL_ROUTE.to_owned(),
        body: JsonObject::new(),
        settings: Settings::default(),
        max_recurse_depth: 100,
    };
    let input = StageInput::AuthPricing(Box::new(stage));
    let Ok(forwarded) = dispatch(AuthBackend::AuthPricing, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production pricing stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthPricing, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));

    // the rule's debug line names the collaborators without printing the caller's token
    let printed = format!("{:?}", PricingRule::default());
    assert!(printed.starts_with("PricingRule {"), "{printed}");
    assert!(printed.contains("has_auth: false"), "{printed}");
    assert!(printed.contains("project_id: None"), "{printed}");
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

fn request_to(reached: &Arc<AtomicBool>, route: &str, body: &'static str) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(route)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(reached));
    request
}

async fn run_rule(
    config: &Config,
    rule: PricingRule,
    route: &str,
    body: &'static str,
) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let application = routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new()
            .route(MODEL_ROUTE, post(mark_called))
            .route(CHAT_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::new(rule)));
    let response = application
        .oneshot(request_to(&reached, route, body))
        .await
        .expect("the request reaches the router");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(bytes.to_vec()).expect("the response body is UTF-8");
    (status, text, reached.load(Ordering::Acquire))
}

async fn run_request(config: &Config, spend: &str) -> (StatusCode, String, bool) {
    let rule = PricingRule {
        auth: None,
        project: None,
        model: RequestedModels::None,
        catalog: None,
        view: None,
        provider: None,
        global_model_aliases: BTreeMap::new(),
        reader: None,
        alerts: None,
        global_spend: Some(spend.parse().expect("the test spend parses")),
    };
    run_rule(config, rule, MODEL_ROUTE, "{}").await
}

#[tokio::test]
async fn failure_stops_downstream() {
    let (status, body, reached) = run_request(&config("5"), "10").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("\"type\":\"budget_exceeded\""), "{body}");
    assert!(
        body.contains("Budget has been exceeded! Current cost: 10, Max budget: 5"),
        "{body}"
    );
    assert!(body.contains("\"param\":null"), "{body}");
    assert!(body.contains("\"code\":\"422\""), "{body}");
    assert!(!reached);

    // a zero ceiling disables the proxy check, so the same spend reaches the handler
    let (off_status, _body, off_reached) = run_request(&config("0"), "10").await;
    assert_eq!(off_status, StatusCode::OK);
    assert!(off_reached);

    // a project budget with no spend reader cannot be verified, so the stage answers 503
    let project: CachedProject = serde_json::from_str(
        r#"{"project_id":"proj-budget","blocked":false,"models":[],
            "litellm_budget_table":{"budget_id":"b1","max_budget":5,"soft_budget":0}}"#,
    )
    .expect("the project row decodes");
    let rule = PricingRule {
        auth: None,
        project: Some(project),
        model: RequestedModels::None,
        catalog: None,
        view: None,
        provider: None,
        global_model_aliases: BTreeMap::new(),
        reader: None,
        alerts: None,
        global_spend: None,
    };
    let (project_status, project_body, project_reached) =
        run_rule(&config("0"), rule, MODEL_ROUTE, "{}").await;
    assert_eq!(project_status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        project_body.contains(BUDGET_UNVERIFIABLE_DETAIL),
        "{project_body}"
    );
    assert!(
        project_body.contains("\"type\":\"auth_error\""),
        "{project_body}"
    );
    assert!(!project_reached);

    // a client-side tags list is a 400 bad_request_error that names the param
    let tags_config = config_with("  reject_clientside_metadata_tags: true\n", "0");
    let (tags_status, tags_body, tags_reached) = run_rule(
        &tags_config,
        PricingRule::default(),
        CHAT_ROUTE,
        r#"{"metadata":{"tags":[]}}"#,
    )
    .await;
    assert_eq!(tags_status, StatusCode::BAD_REQUEST);
    assert!(
        tags_body.contains("\"type\":\"bad_request_error\""),
        "{tags_body}"
    );
    assert!(
        tags_body.contains("\"param\":\"metadata.tags\""),
        "{tags_body}"
    );
    assert!(tags_body.contains("\"code\":\"400\""), "{tags_body}");
    assert!(!tags_reached);

    // a POST chat request without a user is the ordinary 401 auth_error
    let user_config = config_with("  enforce_user_param: true\n", "0");
    let (user_status, user_body, user_reached) =
        run_rule(&user_config, PricingRule::default(), CHAT_ROUTE, "{}").await;
    assert_eq!(user_status, StatusCode::UNAUTHORIZED);
    assert!(user_body.contains("\"type\":\"auth_error\""), "{user_body}");
    assert!(user_body.contains("\"param\":\"None\""), "{user_body}");
    assert!(user_body.contains("\"code\":\"401\""), "{user_body}");
    assert!(!user_reached);
}
