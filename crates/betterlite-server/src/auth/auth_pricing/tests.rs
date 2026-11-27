#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
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
const MASTER_KEY: &str = "secret-master-key";

fn config(max_budget: &str) -> Config {
    let source = format!(
        "general_settings:\n  master_key: {MASTER_KEY}\nlitellm_settings:\n  max_budget: {max_budget}\n"
    );
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source)
    .expect("the test config loads")
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
        .body(Body::from("{}"))
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(reached));
    request
}

async fn run_request(config: &Config, spend: &str) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let rule = Arc::new(PricingRule {
        global_spend: Some(spend.parse().expect("the test spend parses")),
        ..PricingRule::default()
    });
    let application = routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule));
    let response = application
        .oneshot(request_to(&reached))
        .await
        .expect("the request reaches the router");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(bytes.to_vec()).expect("the response body is UTF-8");
    (status, text, reached.load(Ordering::Acquire))
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
}
