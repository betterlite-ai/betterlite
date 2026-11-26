#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_model_budgets::checks::ModelMaxBudget;
use betterlite_auth::auth_model_budgets::limits::{KeyModelBudgetLimiter, UserModelBudgetLimiter};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::auth_model_budgets::Settings;
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{ModelBudgetRule, ModelBudgetStageInput, prefetch};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::auth::rewritten_body;
use crate::routes;
use crate::routes::tests::state_for;

const MODEL_ROUTE: &str = "/v1/files";
const MASTER_KEY: &str = "secret-master-key";

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("the test config loads")
}

fn config_with_master_key() -> Config {
    config("general_settings:\n  master_key: secret-master-key\n")
}

fn key(fields: &str) -> UserAPIKeyAuth {
    let JsonData::Object(map) = serde_json::from_str(fields).expect("the key fixture is JSON")
    else {
        panic!("the key fields must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).expect("the key fixture constructs")
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

// the details every budget rejection in these tests carries, with the entity that owns the counter
fn exceeded_details(entity_type: &str, entity_id: &str) -> BudgetExceededDetails {
    BudgetExceededDetails {
        current_cost: Decimal::new(6, 0),
        max_budget: Decimal::new(5, 0),
        message: Some("Budget has been exceeded!".to_owned()),
        provider: "openai".to_owned(),
        entity_type: Some(entity_type.to_owned()),
        entity_id: Some(entity_id.to_owned()),
        status_code: 422,
    }
}

struct DenyingUserLimiter {
    denial: ErrorClass,
}

impl Default for DenyingUserLimiter {
    fn default() -> Self {
        Self {
            denial: ErrorClass::BudgetExceeded(Box::new(exceeded_details("user", "u1"))),
        }
    }
}

#[async_trait]
impl UserModelBudgetLimiter for DenyingUserLimiter {
    async fn is_user_within_model_budget(
        &self,
        _user_id: &str,
        _user_model_max_budget: &ModelMaxBudget,
        _model: &str,
    ) -> Result<bool, ErrorClass> {
        Err(self.denial.clone())
    }
}

fn failing_rule() -> Arc<ModelBudgetRule> {
    Arc::new(ModelBudgetRule {
        auth: Some(key(
            r#"{"token":"sk-model-budget","user_id":"u1","user_model_max_budget":{"gpt-4":1.0}}"#,
        )),
        models: vec!["gpt-4".to_owned()],
        model_name: None,
        request_model: None,
        user_limiter: Some(Arc::new(DenyingUserLimiter::default())),
        team_limiter: None,
        key_limiter: None,
        global_model_aliases: BTreeMap::new(),
        max_recurse_depth: DEFAULT_RECURSE_DEPTH,
        path_params: None,
        applied: Mutex::new(None),
    })
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

fn application(config: &Config, rule: Arc<ModelBudgetRule>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule))
}

async fn run_request(config: &Config, rule: Arc<ModelBudgetRule>) -> (StatusCode, String, bool) {
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
async fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthModelBudgets));
    // the config section reaches the state slot the stage reads
    let with_settings = config(
        "general_settings:\n  master_key: secret-master-key\nlitellm_settings:\n  max_budget: 12.5\n",
    );
    let state = state_for(&with_settings);
    let settings = Arc::clone(&state.auth_model_budgets.settings);
    assert_eq!(settings.max_budget, Decimal::new(125, 1));
    assert!(settings.max_budget > Decimal::ZERO);

    // the rule's debug output names the type and never the token it resolved
    let debugged = failing_rule();
    assert!(format!("{debugged:?}").contains("ModelBudgetRule"));
    assert!(!format!("{debugged:?}").contains("sk-model-budget"));

    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let stage = ModelBudgetStageInput {
        context: master_context(&master_key),
        rule: failing_rule(),
        settings,
        body: None,
    };
    let input = StageInput::AuthModelBudgets(Box::new(stage));
    let Ok(forwarded) = dispatch(AuthBackend::AuthModelBudgets, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production model budget stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthModelBudgets, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));
}

#[tokio::test]
async fn failure_stops_downstream() {
    let (status, body, reached) = run_request(&config_with_master_key(), failing_rule()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("\"type\":\"budget_exceeded\""), "{body}");
    assert!(body.contains("Budget has been exceeded!"), "{body}");
    assert!(body.contains("\"param\":null"), "{body}");
    assert!(body.contains("\"code\":\"422\""), "{body}");
    assert!(!reached);
}

struct FallbackKeyLimiter;

#[async_trait]
impl KeyModelBudgetLimiter for FallbackKeyLimiter {
    async fn is_key_within_model_budget(
        &self,
        _token: &UserAPIKeyAuth,
        _model: &str,
    ) -> Result<bool, ErrorClass> {
        Err(ErrorClass::BudgetExceeded(Box::new(exceeded_details(
            "key",
            "hashed-key",
        ))))
    }

    async fn fallback_model_within_budget(
        &self,
        _token: &UserAPIKeyAuth,
        _model: &str,
    ) -> Result<Option<String>, ErrorClass> {
        Ok(Some("gpt-3.5".to_owned()))
    }
}

#[tokio::test]
async fn prefetch_rewrites_the_body_for_the_authorized_fallback() {
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let mut path_params = JsonObject::new();
    path_params.insert("model".to_owned(), Json::Text("gpt-4".to_owned()));
    let rule = Arc::new(ModelBudgetRule {
        auth: Some(key(
            r#"{"token":"sk-fallback","models":["gpt-4","gpt-3.5"]}"#,
        )),
        models: vec!["gpt-4".to_owned()],
        model_name: Some("gpt-4".to_owned()),
        request_model: Some("gpt-4".to_owned()),
        user_limiter: None,
        team_limiter: None,
        key_limiter: Some(Arc::new(FallbackKeyLimiter)),
        global_model_aliases: BTreeMap::new(),
        max_recurse_depth: DEFAULT_RECURSE_DEPTH,
        path_params: Some(path_params),
        applied: Mutex::new(None),
    });
    let stage = ModelBudgetStageInput {
        context: master_context(&master_key),
        rule: Arc::clone(&rule),
        settings: Arc::new(Settings::default()),
        body: Some(Bytes::from_static(b"{\"model\":\"gpt-4\"}")),
    };
    prefetch(&stage)
        .await
        .expect("the authorized fallback rewrites the body");
    let applied = rule.applied_body().expect("the fallback body is recorded");
    let body = String::from_utf8(applied.to_vec()).expect("the rewritten body is UTF-8");
    assert_eq!(body, "{\"model\":\"gpt-3.5\"}");
}

#[test]
fn rewritten_body_prefers_the_recorded_fallback() {
    let rule = failing_rule();
    let mut request = request_to(&Arc::new(AtomicBool::new(false)));
    request.extensions_mut().insert(Arc::clone(&rule));
    assert_eq!(rewritten_body(&request, None), None);
    let original = Bytes::from_static(b"{\"model\":\"gpt-4\"}");
    assert_eq!(
        rewritten_body(&request, Some(original.clone())),
        Some(original)
    );
    *rule.applied.lock().unwrap() = Some(Bytes::from_static(b"{\"model\":\"gpt-3.5\"}"));
    assert_eq!(
        rewritten_body(&request, Some(Bytes::from_static(b"{\"model\":\"gpt-4\"}"))),
        Some(Bytes::from_static(b"{\"model\":\"gpt-3.5\"}"))
    );
}
