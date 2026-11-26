#![cfg(test)]

use axum::response::Response;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio_util::task::TaskTracker;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_model_budgets::checks::ModelMaxBudget;
use betterlite_auth::auth_model_budgets::limits::{KeyModelBudgetLimiter, UserModelBudgetLimiter};
use betterlite_auth::auth_model_budgets::request_views::rewrite_model;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::auth_model_budgets::Settings;
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_config::types::Config;
use betterlite_core::api::request_body::ParsedBody;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::body_views::{BodyViews, SharedViews};
use super::{ModelBudgetRule, ModelBudgetStageInput, prefetch};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::auth::rewritten_body;
use crate::routes;
use crate::routes::tests::state_for;

const MODEL_ROUTE: &str = "/v1/chat/completions";
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
        user_limiter: Some(Arc::new(DenyingUserLimiter::default())),
        team_limiter: None,
        key_limiter: None,
        global_model_aliases: BTreeMap::new(),
        max_recurse_depth: DEFAULT_RECURSE_DEPTH,
        path_params: None,
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
    let server = state_for(&with_settings);
    let settings = Arc::clone(&server.auth_model_budgets.settings);
    assert_eq!(settings.max_budget, Decimal::new(125, 1));
    assert!(settings.max_budget > Decimal::ZERO);
    let mut request = request_to(&Arc::new(AtomicBool::new(false)));
    request.extensions_mut().insert(Arc::clone(&server));
    let loaded = super::settings_for_request(&request);
    assert_eq!(loaded.max_budget, Decimal::new(125, 1));

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
        views: SharedViews::default(),
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
        user_limiter: None,
        team_limiter: None,
        key_limiter: Some(Arc::new(FallbackKeyLimiter)),
        global_model_aliases: BTreeMap::new(),
        max_recurse_depth: DEFAULT_RECURSE_DEPTH,
        path_params: Some(path_params),
    });
    let stage = ModelBudgetStageInput {
        context: master_context(&master_key),
        rule: Arc::clone(&rule),
        settings: Arc::new(Settings::default()),
        body: Some(Bytes::from_static(b"{\"model\":\"gpt-4\"}")),
        views: SharedViews::default(),
    };
    prefetch(&stage)
        .await
        .expect("the authorized fallback rewrites the body");
    let views = stage.views.lock().unwrap().clone().unwrap();
    let applied = views
        .serialized_body
        .expect("the fallback body is recorded");
    let body = String::from_utf8(applied).expect("the rewritten body is UTF-8");
    assert_eq!(body, "{\"model\":\"gpt-3.5\"}");
}

#[test]
fn rewritten_body_prefers_the_recorded_fallback() {
    let mut request = request_to(&Arc::new(AtomicBool::new(false)));
    let shared = SharedViews::default();
    request.extensions_mut().insert(Arc::clone(&shared));
    assert_eq!(rewritten_body(&mut request, None), None);
    let original = Bytes::from_static(b"{\"model\":\"gpt-4\"}");
    assert_eq!(
        rewritten_body(&mut request, Some(original.clone())),
        Some(original.clone())
    );
    let mut views = BodyViews::new(Some(&original), None);
    rewrite_model(&mut views, "gpt-3.5").unwrap();
    *shared.lock().unwrap() = Some(views);
    assert_eq!(
        rewritten_body(&mut request, Some(original)),
        Some(Bytes::from_static(b"{\"model\":\"gpt-3.5\"}"))
    );
    let parsed = request.extensions().get::<ParsedBody>().unwrap();
    assert_eq!(
        serde_json::to_string(&parsed.visible()).unwrap(),
        "{\"model\":\"gpt-3.5\"}"
    );
    let snapshots = request.extensions().get::<BodyViews>().unwrap();
    assert_eq!(snapshots.request_data, snapshots.json_cache);
}

async fn echo_fallback(Extension(views): Extension<BodyViews>, body: Bytes) -> Bytes {
    assert_eq!(views.request_data, views.json_cache);
    assert_eq!(
        serde_json::to_vec(&views.parsed_body.unwrap().visible()).unwrap(),
        body
    );
    assert_eq!(
        views.path_params.unwrap().get("model"),
        Some(&Json::Text("gpt-3.5".to_owned()))
    );
    body
}

#[rstest::rstest]
#[case::first(1_i32)]
#[case::second(2_i32)]
#[tokio::test]
async fn fallback_reaches_downstream_with_all_snapshots(#[case] value: i32) {
    let mut rule = fallback_rule(r#"["gpt-4","gpt-3.5"]"#);
    rule.path_params = Some(JsonObject::from([(
        "model".to_owned(),
        Json::Text("gpt-4".to_owned()),
    )]));
    let app = routes::router(
        state_for(&config_with_master_key()),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(echo_fallback)),
        Router::new(),
    )
    .layer(Extension(Arc::new(rule)));
    let mut request = request_to(&Arc::new(AtomicBool::new(false)));
    *request.body_mut() = Body::from(format!(r#"{{"model":"gpt-4","value":{value}}}"#));
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(bytes, format!(r#"{{"model":"gpt-3.5","value":{value}}}"#));
}

#[rstest::rstest]
#[case::allowed(r#"["gpt-4","gpt-3.5"]"#, "200")]
#[case::denied(r#"["gpt-4"]"#, "422")]
#[tokio::test]
async fn booted_chat_fallback_and_denial(#[case] models: &str, #[case] status: &str) {
    let mut rule = fallback_rule(models);
    rule.path_params = Some(JsonObject::from([(
        "model".to_owned(),
        Json::Text("gpt-4".to_owned()),
    )]));
    let app = routes::router(
        state_for(&config_with_master_key()),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(echo_fallback)),
        Router::new(),
    )
    .layer(Extension(Arc::new(rule)));
    let response = exchange(app).await;
    assert!(
        response.starts_with(&format!("HTTP/1.1 {status} ")),
        "{response}"
    );
    if status == "200" {
        assert!(response.ends_with(r#"{"model":"gpt-3.5"}"#), "{response}");
    } else {
        assert!(
            response.contains(r#""type":"budget_exceeded""#),
            "{response}"
        );
        assert!(!response.contains("gpt-3.5"), "{response}");
    }
}

#[test]
fn error_translation_preserves_fields_and_default_message() {
    use betterlite_auth::auth_model_budgets::fallback::KeyBudgetError;
    use betterlite_auth::auth_model_budgets::request_views::RewriteFailure;
    let mut details = exceeded_details("key", "key");
    details.message = None;
    let exception = super::key_budget_exception(&KeyBudgetError::Budget(Box::new(details)));
    assert_eq!(exception.code, "422");
    assert_eq!(exception.error_type, "budget_exceeded");
    assert_eq!(exception.param, None);
    assert_eq!(
        exception.message,
        "Budget has been exceeded! Current cost: 6, Max budget: 5"
    );
    let other = super::key_budget_exception(&KeyBudgetError::Rewrite(RewriteFailure));
    assert_eq!(other.code, "401");
    assert_eq!(other.error_type, "auth_error");
    assert_eq!(other.param.as_deref(), Some("None"));
    assert!(other.message.contains("could not be serialized"));
}

#[test]
fn fallback_does_not_create_a_path_model() {
    let original = Bytes::from_static(b"{\"model\":\"gpt-4\"}");
    let mut views = BodyViews::new(Some(&original), Some(JsonObject::new()));
    rewrite_model(&mut views, "fallback").unwrap();
    assert_eq!(views.path_params, Some(JsonObject::new()));
    let mut absent = BodyViews::new(Some(&original), None);
    rewrite_model(&mut absent, "fallback").unwrap();
    assert_eq!(absent.path_params, None);
}

#[tokio::test]
async fn slot_limiter_is_used_when_request_has_no_override() {
    let mut state = state_for(&config_with_master_key());
    Arc::get_mut(&mut state)
        .unwrap()
        .auth_model_budgets
        .user_limiter = Some(Arc::new(DenyingUserLimiter::default()));
    let mut rule = (*failing_rule()).clone();
    rule.user_limiter = None;
    let app = routes::router(
        state,
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::new(rule)));
    let reached = Arc::new(AtomicBool::new(false));
    let response = app.oneshot(request_to(&reached)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!reached.load(Ordering::Acquire));
}

#[derive(serde::Deserialize)]
struct ProviderRequest {
    model: String,
}

async fn mock_provider(
    Extension(models): Extension<Arc<Mutex<Vec<String>>>>,
    body: Bytes,
) -> &'static str {
    let request: ProviderRequest = serde_json::from_slice(&body).unwrap();
    models.lock().unwrap().push(request.model);
    r#"{"id":"c1","object":"chat.completion","created":7,"model":"fallback-wire","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#
}

#[rstest::rstest]
#[case::allowed(r#"["gpt-4","gpt-3.5"]"#, StatusCode::OK, vec!["fallback-wire"])]
#[case::denied(r#"["gpt-4"]"#, StatusCode::UNPROCESSABLE_ENTITY, vec![])]
#[tokio::test]
async fn production_chat_sends_authorized_fallback_to_provider(
    #[case] models: &str,
    #[case] expected_status: StatusCode,
    #[case] expected_models: Vec<&str>,
) {
    use tokio::{net::TcpListener, task::JoinSet};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let sent = Arc::new(Mutex::new(Vec::<String>::new()));
    let provider = Router::new()
        .fallback(mock_provider)
        .layer(Extension(Arc::clone(&sent)));
    let mut servers = JoinSet::new();
    servers.spawn(async move {
        axum::serve(listener, provider).await.unwrap();
    });
    let tasks = TaskTracker::new();
    let app = chat_app(address, &tasks, models);
    let response = call_chat(app).await;
    assert_eq!(response.status(), expected_status);
    if expected_status == StatusCode::OK {
        assert_eq!(
            response.headers().get("x-litellm-model-group").unwrap(),
            "gpt-3.5"
        );
    }
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    if expected_status == StatusCode::UNPROCESSABLE_ENTITY {
        assert!(
            String::from_utf8(bytes.to_vec())
                .unwrap()
                .contains("budget_exceeded")
        );
    }
    assert_eq!(*sent.lock().unwrap(), expected_models);

    servers.shutdown().await;
    tasks.close();
    tasks.wait().await;
}

async fn exchange(app: Router) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::task::JoinSet;
    use tokio::time::{Duration, timeout};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut servers = JoinSet::new();
    servers.spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let response = timeout(Duration::from_secs(5), async {
        let mut connection = TcpStream::connect(address).await.unwrap();
        let body = r#"{"model":"gpt-4"}"#;
        let wire = format!("POST {MODEL_ROUTE} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {MASTER_KEY}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        connection.write_all(wire.as_bytes()).await.unwrap();
        let mut answer = String::new();
        connection.take(8192).read_to_string(&mut answer).await.unwrap();
        answer
    }).await.unwrap();
    servers.abort_all();
    while servers.join_next().await.is_some() {}
    response
}

fn fallback_rule(models: &str) -> ModelBudgetRule {
    ModelBudgetRule {
        auth: Some(key(&format!(r#"{{"models":{models}}}"#))),
        models: vec!["gpt-4".to_owned()],
        model_name: Some("gpt-4".to_owned()),
        key_limiter: Some(Arc::new(FallbackKeyLimiter)),
        user_limiter: None,
        team_limiter: None,
        global_model_aliases: BTreeMap::new(),
        max_recurse_depth: DEFAULT_RECURSE_DEPTH,
        path_params: None,
    }
}

fn chat_app(address: SocketAddr, tasks: &TaskTracker, models: &str) -> Router {
    use crate::server::{application as server_application, chat_setup::chat_executor};
    use rand::{SeedableRng as _, rngs::StdRng};
    let source = format!(
        "general_settings: {{master_key: {MASTER_KEY}}}\nmodel_list:\n  - {{model_name: gpt-3.5, litellm_params: {{model: openai/fallback-wire, api_key: dummy, api_base: 'http://{address}/v1'}}}}\n"
    );
    let loaded = config(&source);
    let executor = chat_executor(&loaded, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
    server_application(&loaded, executor, Arc::new(AtomicBool::new(false)), tasks)
        .unwrap()
        .layer(Extension(Arc::new(fallback_rule(models))))
}

async fn call_chat(app: Router) -> Response {
    let request = Request::post(MODEL_ROUTE)
        .header(header::AUTHORIZATION, format!("Bearer {MASTER_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}]}"#,
        ))
        .unwrap();
    app.oneshot(request).await.unwrap()
}
