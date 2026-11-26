#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_request_models::catalog::{
    CatalogError, DeploymentRecord, ModelCatalog, RequestModelResolver,
};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use tower::ServiceExt as _;

use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageInput, dispatch,
};
use crate::auth::auth_request_models::CoreDecoders;
use crate::auth::stage_input;
use crate::routes;
use crate::routes::tests::state_for;

const MASTER_KEY: &str = "secret-master-key";
// the prediction route is the one model source that reads catalog deployments from the request body
const PREDICTION_ROUTE: &str = "/cost/predict-cache";
const PREDICTION_BODY: &str =
    r#"{"current_deployment_id": "current-id", "candidate_deployment_id": "candidate-id"}"#;

#[derive(Debug)]
struct FailingCatalog;

impl ModelCatalog for FailingCatalog {
    fn resolve_model_name(&self, _model_id: &str) -> Result<Option<String>, CatalogError> {
        Err(CatalogError::InvalidDeployment)
    }

    fn deployment(&self, _deployment_id: &str) -> Result<Option<DeploymentRecord>, CatalogError> {
        Err(CatalogError::InvalidDeployment)
    }

    fn model_names(&self) -> Result<BTreeSet<String>, CatalogError> {
        Err(CatalogError::InvalidDeployment)
    }

    fn model_list(&self) -> Result<Vec<DeploymentRecord>, CatalogError> {
        Err(CatalogError::InvalidDeployment)
    }
}

fn config_with_master_key() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!("general_settings:\n  master_key: {MASTER_KEY}\n"))
    .expect("the test config loads")
}

fn master_context(master_key: &SecretString) -> AuthContext {
    let Ok(context) = dispatch(
        AuthBackend::MasterKey,
        &StageInput::MasterKey(Some(master_key.clone())),
        master_key,
    ) else {
        panic!("the master key stage authenticates")
    };
    context
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthContextSources));
    let config = config_with_master_key();
    let app = state_for(&config);
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"model": "gpt-4"}"#))
        .expect("the request builds");
    let body = Bytes::from_static(br#"{"model": "gpt-4"}"#);
    let input = stage_input(
        AuthBackend::AuthContextSources,
        &request,
        Some(&master_context(&master_key)),
        &app.auth_request_models.resolver,
        Some(&body),
    )
    .unwrap_or_else(|| panic!("the request context stage input builds"));
    let StageInput::AuthContextSources(stage) = &input else {
        panic!("the input carries the request model stage")
    };
    assert_eq!(stage.route, "/v1/chat/completions");
    assert_eq!(
        stage.body.get("model"),
        Some(&Json::Text("gpt-4".to_owned()))
    );
    let Ok(forwarded) = dispatch(AuthBackend::AuthContextSources, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production context stage")
    };
    assert_eq!(forwarded.principal.subject, "master_key");
}

async fn mark_reached(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

fn prediction_request(
    reached: &Arc<AtomicBool>,
    resolver: Option<Arc<RequestModelResolver>>,
) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(PREDICTION_ROUTE)
        .header(header::AUTHORIZATION, format!("Bearer {MASTER_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(PREDICTION_BODY))
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(reached));
    if let Some(custom) = resolver {
        request.extensions_mut().insert(custom);
    }
    request
}

fn failing_resolver() -> Arc<RequestModelResolver> {
    Arc::new(RequestModelResolver {
        catalog: Arc::new(FailingCatalog),
        decoders: Arc::new(CoreDecoders),
    })
}

#[tokio::test]
async fn failure_stops_downstream() {
    let config = config_with_master_key();
    let reached = Arc::new(AtomicBool::new(false));
    let chat = Router::new().route(PREDICTION_ROUTE, post(mark_reached));
    let application = routes::router(
        state_for(&config),
        Arc::new(AtomicBool::new(false)),
        chat,
        Router::new(),
    );
    let passing = application
        .clone()
        .oneshot(prediction_request(&reached, None))
        .await
        .expect("the request reaches the router");
    assert_eq!(passing.status(), StatusCode::OK);
    assert!(reached.load(Ordering::Acquire));
    reached.store(false, Ordering::Release);
    let response = application
        .oneshot(prediction_request(&reached, Some(failing_resolver())))
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the error body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the error body is UTF-8");
    assert!(
        text.contains(
            "\"message\":\"Authentication Error, the deployment is stored in an invalid format\""
        ),
        "{text}"
    );
    assert!(text.contains("\"type\":\"auth_error\""), "{text}");
    assert!(text.contains("\"param\":\"None\""), "{text}");
    assert!(text.contains("\"code\":\"401\""), "{text}");
    assert!(!reached.load(Ordering::Acquire));
}
