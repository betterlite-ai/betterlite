#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{ResourceGrantRule, ResourceGrantStageInput};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";

const RESTRICTED_KEY: &str = r#"{"token":"key-1","object_permission_id":"perm-key","object_permission":{"object_permission_id":"perm-key","search_tools":["allowed-tool"]}}"#;

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

fn key(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fields decode");
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs")
}

fn permission(row: &str) -> LiteLLMObjectPermissionTable {
    serde_json::from_str(row).expect("the permission row decodes")
}

fn permission_rows() -> BTreeMap<String, LiteLLMObjectPermissionTable> {
    let mut rows = BTreeMap::new();
    rows.insert(
        "perm-key".to_owned(),
        permission(r#"{"object_permission_id":"perm-key","vector_stores":["vs-allowed"]}"#),
    );
    rows
}

fn rule(
    search_tool: Option<&str>,
    vector_ids: &[&str],
    rag: Option<&str>,
) -> Arc<ResourceGrantRule> {
    Arc::new(ResourceGrantRule {
        key: Some(key(RESTRICTED_KEY)),
        team: None,
        object_permissions: permission_rows(),
        search_tool_name: search_tool.map(str::to_owned),
        vector_store_ids: vector_ids.iter().map(|id| (*id).to_owned()).collect(),
        rag_vector_store_id: rag.map(str::to_owned),
    })
}

fn search_rule(asked: &str) -> Arc<ResourceGrantRule> {
    rule(Some(asked), &[], None)
}

fn vector_rule(ids: &[&str]) -> Arc<ResourceGrantRule> {
    rule(None, ids, None)
}

fn rag_rule(id: &str) -> Arc<ResourceGrantRule> {
    rule(None, &[], Some(id))
}

fn allowed_rule() -> Arc<ResourceGrantRule> {
    rule(Some("allowed-tool"), &["vs-allowed"], None)
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthResourceGrants));
    // LiteLLM runs the vector store check only when a database is configured
    let without = state_for(&config_with_master_key());
    assert!(!without.auth_resource_grants.database);
    let with = state_for(&config_with_database());
    assert!(with.auth_resource_grants.database);

    let master_key = SecretString::from("secret-master-key".to_owned());
    let granted = ResourceGrantStageInput {
        context: master_context(&master_key),
        rule: allowed_rule(),
        database: true,
    };
    let granted_input = StageInput::AuthResourceGrants(Box::new(granted));
    let Ok(forwarded) = dispatch(AuthBackend::AuthResourceGrants, &granted_input, &master_key)
    else {
        panic!("the B732 constructor dispatches to the production resource grant stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let refused = ResourceGrantStageInput {
        context: master_context(&master_key),
        rule: search_rule("other"),
        database: false,
    };
    let refused_input = StageInput::AuthResourceGrants(Box::new(refused));
    assert!(matches!(
        dispatch(AuthBackend::AuthResourceGrants, &refused_input, &master_key),
        Err(StageFailure::ResourceGrants(_))
    ));

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthResourceGrants, &wrong_stage, &master_key),
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

fn application(config: &Config, rule: Arc<ResourceGrantRule>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule))
}

async fn run_request(config: &Config, rule: Arc<ResourceGrantRule>) -> (StatusCode, String, bool) {
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
    let (status, body, reached) = run_request(&config_with_database(), search_rule("other")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body.contains("\"type\":\"key_model_access_denied\""),
        "{body}"
    );
    assert!(body.contains("\"param\":\"search_tool_name\""), "{body}");
    assert!(body.contains("\"code\":\"403\""), "{body}");
    assert!(
        body.contains("Key not allowed to access search tool: other."),
        "{body}"
    );
    assert!(
        body.contains("Allowed search tools: allowed-tool"),
        "{body}"
    );
    assert!(!reached);

    let (vector_status, vector_body, vector_reached) =
        run_request(&config_with_database(), vector_rule(&["vs-1"])).await;
    assert_eq!(vector_status, StatusCode::UNAUTHORIZED);
    assert!(
        vector_body.contains("\"type\":\"key_vector_store_access_denied\""),
        "{vector_body}"
    );
    assert!(!vector_reached);
}

#[tokio::test]
async fn vector_denial_carries_its_fields() {
    let (status, body, reached) =
        run_request(&config_with_database(), vector_rule(&["vs-1"])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("\"param\":\"vector_store\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(
        body.contains("User not allowed to access vector store. Tried to access vs-1."),
        "{body}"
    );
    assert!(body.contains("Only allowed to access vs-allowed"), "{body}");
    assert!(!reached);
}

// the RAG retrieval id reaches the check even without a registry selector
#[tokio::test]
async fn rag_permission_stops_downstream() {
    let (status, _body, reached) = run_request(&config_with_database(), rag_rule("vs-1")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!reached);
}

#[tokio::test]
async fn permitted_requests_reach_the_handler() {
    let (rag_status, _rag_body, rag_reached) =
        run_request(&config_with_database(), rag_rule("vs-allowed")).await;
    assert_eq!(rag_status, StatusCode::OK);
    assert!(rag_reached);

    let (allowed_status, _allowed_body, allowed_reached) =
        run_request(&config_with_database(), allowed_rule()).await;
    assert_eq!(allowed_status, StatusCode::OK);
    assert!(allowed_reached);

    // without a database LiteLLM never reads the permission rows, so the same rule passes
    let (skip_status, _skip_body, skip_reached) =
        run_request(&config_with_master_key(), vector_rule(&["vs-1"])).await;
    assert_eq!(skip_status, StatusCode::OK);
    assert!(skip_reached);
}
