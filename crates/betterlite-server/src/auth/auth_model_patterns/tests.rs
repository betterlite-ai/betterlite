#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_model_patterns::grant::ModelGrantPattern;
use betterlite_auth::auth_request_models::{MODEL_ROUTING_HEADER_NAME, RequestedModels};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_error::proxy_error_types::ModelAccessObject;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{ModelPatternRule, ModelPatternStageInput};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

fn config_with_master_key(master_key: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!("general_settings:\n  master_key: {master_key}\n"))
    .expect("the test config loads")
}

fn master_context(master_key: &SecretString) -> AuthContext {
    let input = StageInput::MasterKey(Some(master_key.clone()));
    let Ok(context) = dispatch(AuthBackend::MasterKey, &input, master_key) else {
        panic!("the master key stage authenticates");
    };
    context
}

fn rule(models: &[&str]) -> Arc<ModelPatternRule> {
    Arc::new(ModelPatternRule {
        object: ModelAccessObject::Key,
        grant: ModelGrantPattern {
            models: models.iter().map(|model| (*model).to_owned()).collect(),
            team_model_aliases: BTreeMap::new(),
            key_model_aliases: BTreeMap::new(),
            team_id: None,
        },
        view: None,
        provider: None,
        global_model_aliases: BTreeMap::new(),
    })
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthModelPatterns));
    let config = config_with_master_key("secret-master-key");
    let state = state_for(&config);
    // the stage enforces the depth the server constructor resolved from the config
    assert_eq!(state.auth_model_patterns.max_recurse_depth, 100);
    let master_key = SecretString::from("secret-master-key".to_owned());
    let stage = ModelPatternStageInput {
        context: master_context(&master_key),
        requested: RequestedModels::One("gpt-4o".to_owned()),
        rule: rule(&["gpt-4o"]),
        max_recurse_depth: state.auth_model_patterns.max_recurse_depth,
    };
    let input = StageInput::AuthModelPatterns(Box::new(stage));
    let Ok(forwarded) = dispatch(AuthBackend::AuthModelPatterns, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production model pattern stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let denied = ModelPatternStageInput {
        context: master_context(&master_key),
        requested: RequestedModels::One("internal-model".to_owned()),
        rule: rule(&["gpt-4o"]),
        max_recurse_depth: state.auth_model_patterns.max_recurse_depth,
    };
    let denied_input = StageInput::AuthModelPatterns(Box::new(denied));
    assert!(matches!(
        dispatch(AuthBackend::AuthModelPatterns, &denied_input, &master_key),
        Err(StageFailure::ModelAccess(_))
    ));
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

fn request_to(reached: &Arc<AtomicBool>, model: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/auth-test/files")
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        // the model routing header is the requested model on this route
        .header(MODEL_ROUTING_HEADER_NAME, model)
        .body(Body::empty())
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(reached));
    request
}

fn application(models: &[&str]) -> Router {
    let config = config_with_master_key("secret-master-key");
    routes::router(
        state_for(&config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route("/auth-test/files", post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule(models)))
}

#[tokio::test]
async fn failure_stops_downstream() {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(&["gpt-4o"])
        .oneshot(request_to(&reached, "internal-model"))
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the error body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the error body is UTF-8");
    assert!(
        text.contains("\"type\":\"key_model_access_denied\""),
        "{text}"
    );
    assert!(text.contains("\"param\":\"model\""), "{text}");
    assert!(text.contains("\"code\":\"403\""), "{text}");
    assert!(
        text.contains("The requested model 'internal-model' is not available for this API key"),
        "{text}"
    );
    // the allowlist stays in the diagnostics, never in the client body
    assert!(!text.contains("gpt-4o"), "{text}");
    assert!(!reached.load(Ordering::Acquire));
}

#[tokio::test]
async fn allowed_model_reaches_the_route() {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(&["gpt-4o"])
        .oneshot(request_to(&reached, "gpt-4o"))
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(reached.load(Ordering::Acquire));
}
