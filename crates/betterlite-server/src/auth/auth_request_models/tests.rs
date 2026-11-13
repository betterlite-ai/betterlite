#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use betterlite_auth::auth_request_models::catalog::{
    CatalogError, Collaborators, DeploymentRecord, ModelCatalog, RequestModelResolver,
};
use betterlite_auth::auth_request_models::sources::{
    DispatchContext, RequestSnapshot, extract_model_candidates_from_request,
};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_models::model_registration::JsonObject;
use secrecy::SecretString;
use tower::ServiceExt as _;

use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

use super::{CoreDecoders, RequestModelStageInput};

#[derive(Debug, Default)]
struct FakeCatalog {
    resolved: BTreeMap<String, String>,
    failing: bool,
}

impl ModelCatalog for FakeCatalog {
    fn resolve_model_name_from_model_id(
        &self,
        model_id: &str,
    ) -> Result<Option<String>, CatalogError> {
        if self.failing {
            return Err(CatalogError::InvalidDeployment);
        }
        Ok(self.resolved.get(model_id).cloned())
    }

    fn deployment(&self, _deployment_id: &str) -> Result<Option<DeploymentRecord>, CatalogError> {
        Ok(None)
    }

    fn model_names(&self) -> Result<BTreeSet<String>, CatalogError> {
        if self.failing {
            return Err(CatalogError::InvalidDeployment);
        }
        Ok(BTreeSet::new())
    }

    fn model_list(&self) -> Result<Vec<DeploymentRecord>, CatalogError> {
        Ok(Vec::new())
    }
}

#[derive(Debug)]
struct FailingCatalog;

impl ModelCatalog for FailingCatalog {
    fn resolve_model_name_from_model_id(
        &self,
        _model_id: &str,
    ) -> Result<Option<String>, CatalogError> {
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

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture parses")
}

fn config_with_master_key(master_key: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!("general_settings:\n  master_key: {master_key}\n"))
    .expect("the test config loads")
}

fn master_context(master_key: &SecretString) -> AuthContext {
    dispatch(
        AuthBackend::MasterKey,
        &StageInput::MasterKey(Some(master_key.clone())),
        master_key,
    )
    .expect("the master key stage authenticates")
}

fn stage_input(
    context: AuthContext,
    route: &str,
    body: &str,
    resolver: Arc<RequestModelResolver>,
) -> RequestModelStageInput {
    RequestModelStageInput {
        context,
        route: route.to_owned(),
        body: object(body),
        headers: None,
        query: None,
        endpoint_marker: None,
        path_params: BTreeSet::new(),
        team_id: None,
        resolver,
    }
}

fn resolver_with(catalog: FakeCatalog) -> Arc<RequestModelResolver> {
    Arc::new(RequestModelResolver {
        catalog: Arc::new(catalog),
        decoders: Arc::new(CoreDecoders),
    })
}

fn candidates_with_catalog(body: &JsonObject, catalog: &dyn ModelCatalog) -> Vec<String> {
    let path_params = BTreeSet::new();
    let request = RequestSnapshot {
        route: "/files",
        body,
        headers: None,
        query: None,
        dispatch: DispatchContext {
            endpoint_marker: None,
            path_params: &path_params,
        },
        team_id: None,
    };
    let decoders = CoreDecoders;
    let collaborators = Collaborators {
        catalog: Some(catalog),
        decoders: &decoders,
    };
    extract_model_candidates_from_request(&request, &collaborators)
        .expect("the catalog lookup succeeds")
}

fn managed(text: &str) -> String {
    URL_SAFE_NO_PAD.encode(text)
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthRequestModels));
    let config = config_with_master_key("secret-master-key");
    let state = state_for(&config);
    let master_key = SecretString::from("secret-master-key".to_owned());
    let stage = stage_input(
        master_context(&master_key),
        "/v1/chat/completions",
        r#"{"model": "gpt-4"}"#,
        state.auth_request_models.resolver(),
    );
    let input = StageInput::AuthRequestModels(Box::new(stage));
    let forwarded = dispatch(AuthBackend::AuthRequestModels, &input, &master_key)
        .expect("the B732 constructor dispatches to the production model stage");
    assert_eq!(forwarded.principal.subject, "master_key");
    let failing = Arc::new(RequestModelResolver {
        catalog: Arc::new(FailingCatalog),
        decoders: Arc::new(CoreDecoders),
    });
    let stage = stage_input(
        master_context(&master_key),
        "/azure/gpt-4/chat/completions",
        r#"{"model": "gpt-4"}"#,
        failing,
    );
    let failing_input = StageInput::AuthRequestModels(Box::new(stage));
    assert!(dispatch(AuthBackend::AuthRequestModels, &failing_input, &master_key).is_err());
}

async fn mark_reached(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

fn azure_request(
    reached: &Arc<AtomicBool>,
    resolver: Option<Arc<RequestModelResolver>>,
) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/azure/gpt-4/chat/completions")
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"model": "gpt-4"}"#))
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(reached));
    if let Some(resolver) = resolver {
        request.extensions_mut().insert(resolver);
    }
    request
}

#[tokio::test]
async fn failure_stops_downstream() {
    let config = config_with_master_key("secret-master-key");
    let reached = Arc::new(AtomicBool::new(false));
    let chat = Router::new().route("/azure/{model}/chat/completions", post(mark_reached));
    let application = routes::router(
        state_for(&config),
        Arc::new(AtomicBool::new(false)),
        chat,
        Router::new(),
    );
    let passing = application
        .clone()
        .oneshot(azure_request(&reached, None))
        .await
        .expect("the request reaches the router");
    assert_eq!(passing.status(), StatusCode::OK);
    assert!(reached.load(Ordering::Acquire));
    reached.store(false, Ordering::Release);
    let failing = Arc::new(RequestModelResolver {
        catalog: Arc::new(FailingCatalog),
        decoders: Arc::new(CoreDecoders),
    });
    let response = application
        .oneshot(azure_request(&reached, Some(failing)))
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the error body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the error body is UTF-8");
    assert_eq!(text, "Internal Server Error");
    assert!(!reached.load(Ordering::Acquire));
}

#[test]
fn legacy_then_unified_then_video() {
    let legacy = format!("file-{}", managed("litellm:file-1;model,legacy-model"));
    let unified = managed("litellm_proxy:file;unified_id,u;target_model_names,a,b;model_id,mid");
    let video = format!(
        "video_{}",
        managed("litellm:custom_llm_provider:az;model_id:video-model;video_id:v1")
    );
    let body = object(&format!(
        r#"{{"file_id": "{legacy}", "batch_id": "{unified}", "video_id": "{video}"}}"#
    ));
    let catalog = FakeCatalog {
        resolved: BTreeMap::from([
            ("mid".to_owned(), "unified-group".to_owned()),
            ("video-model".to_owned(), "video-group".to_owned()),
        ]),
        failing: false,
    };
    assert_eq!(
        candidates_with_catalog(&body, &catalog),
        ["legacy-model", "a", "b", "unified-group", "video-group"]
    );
}

#[test]
fn malformed_id_next_decoder() {
    let legacy_only = format!("file-{}", managed("litellm:file-1;model,legacy-model"));
    let unified_only = managed("litellm_proxy:file;unified_id,u;model_id,mid");
    let body = object(&format!(
        r#"{{"file_id": "{legacy_only}", "batch_id": "{unified_only}"}}"#
    ));
    let catalog = FakeCatalog {
        resolved: BTreeMap::from([("mid".to_owned(), "unified-group".to_owned())]),
        failing: false,
    };
    assert_eq!(
        candidates_with_catalog(&body, &catalog),
        ["legacy-model", "unified-group"]
    );
}

#[test]
fn unknown_model_id_kept() {
    let video = format!(
        "video_{}",
        managed("litellm:custom_llm_provider:az;model_id:unknown-model;video_id:v1")
    );
    let body = object(&format!(r#"{{"video_id": "{video}"}}"#));
    let catalog = FakeCatalog::default();
    assert_eq!(candidates_with_catalog(&body, &catalog), ["unknown-model"]);
}

#[test]
fn resolver_error_debug_kept() {
    let video = format!(
        "video_{}",
        managed("litellm:custom_llm_provider:az;model_id:unknown-model;video_id:v1")
    );
    let body = object(&format!(r#"{{"video_id": "{video}"}}"#));
    let catalog = FakeCatalog {
        resolved: BTreeMap::new(),
        failing: true,
    };
    assert_eq!(candidates_with_catalog(&body, &catalog), ["unknown-model"]);
}

#[test]
fn video_character_field_specific() {
    let video = format!(
        "video_{}",
        managed("litellm:custom_llm_provider:az;model_id:video-model;video_id:v1")
    );
    let character = format!(
        "character_{}",
        managed("litellm:custom_llm_provider:az;model_id:character-model;character_id:c1")
    );
    let catalog = FakeCatalog {
        resolved: BTreeMap::from([
            ("video-model".to_owned(), "video-group".to_owned()),
            ("character-model".to_owned(), "character-group".to_owned()),
        ]),
        failing: false,
    };
    let paired = object(&format!(
        r#"{{"video_id": "{video}", "character_id": "{character}"}}"#
    ));
    assert_eq!(
        candidates_with_catalog(&paired, &catalog),
        ["video-group", "character-group"]
    );
    let swapped = object(&format!(
        r#"{{"video_id": "{character}", "character_id": "{video}"}}"#
    ));
    assert!(candidates_with_catalog(&swapped, &catalog).is_empty());
}

#[test]
fn empty_and_nonstring_ids_yield_no_hints() {
    let body = object(r#"{"file_id": "", "batch_id": 7, "video_id": null}"#);
    assert!(candidates_with_catalog(&body, &FakeCatalog::default()).is_empty());
}
