#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_cache::{AuthCacheError, WriteTtl};
use betterlite_auth::auth_identity::records::SourceKey;
use betterlite_auth::end_user_identity::cache::ValidationCache;
use betterlite_auth::end_user_identity::existence::{
    EndUserExistence, ExistenceError, UserExistence, UserProbe,
};
use betterlite_auth::end_user_identity::resolve::{DatabaseLookups, EndUserIdResolver};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::PydanticBool;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_models::decisions::Json;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;
use tower::ServiceExt as _;

use super::{
    EndUserIdentityRule, EndUserIdentityStageInput, installed_resolver, prefetch, prefetch_budget,
    stage_input,
};
use crate::auth::auth_end_users::{self, EndUserFailure, EndUserLookup, EndUserRule};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;
use crate::server::application as server_application;
use crate::server::chat_setup::chat_executor;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";

#[derive(Debug)]
struct ScriptedCache {
    failing: bool,
    reads: Mutex<Vec<String>>,
}

impl ScriptedCache {
    fn new(failing: bool) -> Arc<Self> {
        Arc::new(Self {
            failing,
            reads: Mutex::new(Vec::new()),
        })
    }

    fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("the cache reads").clone()
    }
}

fn cache_failure() -> AuthCacheError {
    let source = serde_json::from_str::<i64>("not a number").expect_err("the invalid json");
    AuthCacheError::Payload(Box::new(PayloadError::Json(source)))
}

#[async_trait]
impl ValidationCache for ScriptedCache {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.reads
            .lock()
            .expect("the cache reads")
            .push(cache_key.to_owned());
        if self.failing {
            return Err(cache_failure());
        }
        Ok(None)
    }

    async fn write(
        &self,
        _cache_key: &str,
        _value: &Json,
        _ttl: WriteTtl,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }
}

#[derive(Debug)]
struct ScriptedExistence {
    found: bool,
    calls: Mutex<Vec<String>>,
}

impl ScriptedExistence {
    fn new(found: bool) -> Arc<Self> {
        Arc::new(Self {
            found,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("the existence calls").clone()
    }
}

#[async_trait]
impl EndUserExistence for ScriptedExistence {
    async fn end_user_exists(&self, end_user_id: &str) -> Result<bool, ExistenceError> {
        self.calls
            .lock()
            .expect("the existence calls")
            .push(end_user_id.to_owned());
        Ok(self.found)
    }
}

#[async_trait]
impl UserExistence for ScriptedExistence {
    async fn user_exists(&self, probe: &UserProbe<'_>) -> Result<bool, ExistenceError> {
        self.calls
            .lock()
            .expect("the existence calls")
            .push(probe.user_id.to_owned());
        Ok(self.found)
    }
}

// the production resolver over the scripted ports, with the found end user recorded for assertions
fn resolver(
    found: bool,
    cache: &Arc<ScriptedCache>,
) -> (Arc<EndUserIdResolver>, Arc<ScriptedExistence>) {
    let end_users = ScriptedExistence::new(found);
    let users = ScriptedExistence::new(false);
    let identity = EndUserIdResolver {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: None,
        lookups: Some(DatabaseLookups {
            end_users: Arc::<ScriptedExistence>::clone(&end_users),
            users,
            deadline_seconds: 1.0_f64,
        }),
        cache: Arc::<ScriptedCache>::clone(cache),
    };
    (Arc::new(identity), end_users)
}

fn config_with_master_key() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .expect("the test config loads")
}

// the database with litellm_settings.validate_end_user_id_in_db under the caller's control
fn config_with_database(validate_end_user_id_in_db: bool) -> Config {
    let validation = if validate_end_user_id_in_db {
        "litellm_settings:\n  validate_end_user_id_in_db: true\n"
    } else {
        ""
    };
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!(
        "general_settings:\n  master_key: secret-master-key\n  database_url: postgresql://user:password@127.0.0.1:1/betterlite\n{validation}"
    ))
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
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthEndUserIdentity));
    let identity = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::AuthEndUserIdentity)
        .expect("the identity stage is installed");
    let end_users = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::AuthEndUsers)
        .expect("the end-user stage is installed");
    assert!(
        identity < end_users,
        "the id is validated before the end-user lookup reads it"
    );
    let without = state_for(&config_with_master_key());
    assert!(!without.end_user_identity.database);
    assert!(!without.end_user_identity.validate_end_user_id_in_db);
    assert!(without.end_user_identity.max_end_user_budget_id.is_none());
    let with = state_for(&config_with_database(true));
    assert!(with.end_user_identity.database);
    assert!(with.end_user_identity.validate_end_user_id_in_db);

    let master_key = SecretString::from("secret-master-key".to_owned());
    let stage = EndUserIdentityStageInput {
        context: master_context(&master_key),
        raw_end_user_id: Some("alice".to_owned()),
        rule: Arc::new(EndUserIdentityRule::default()),
        database: true,
        state: None,
        tasks: None,
    };
    let input = StageInput::AuthEndUserIdentity(Box::new(stage));
    let Ok(forwarded) = dispatch(AuthBackend::AuthEndUserIdentity, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production end-user identity stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");
    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthEndUserIdentity, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));
}

fn stage_request(headers: &[(&str, &str)], body: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::CONTENT_TYPE, "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request
        .body(Body::from(body.to_owned()))
        .expect("the request builds")
}

fn extracted(config: &Config, headers: &[(&str, &str)], body: &str) -> Option<String> {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let context = master_context(&master_key);
    let mut request = stage_request(headers, body);
    request.extensions_mut().insert(state_for(config));
    let bytes = Bytes::from(body.to_owned());
    let Ok(StageInput::AuthEndUserIdentity(stage)) =
        stage_input(&request, Some(&context), Some(&bytes))
    else {
        panic!("the authenticated request builds its stage input");
    };
    stage.raw_end_user_id
}

#[test]
fn database_flag_comes_from_the_state() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let context = master_context(&master_key);
    let mut without = stage_request(&[], "{}");
    without
        .extensions_mut()
        .insert(state_for(&config_with_master_key()));
    let Ok(StageInput::AuthEndUserIdentity(without_stage)) =
        stage_input(&without, Some(&context), None)
    else {
        panic!("the authenticated request builds its stage input");
    };
    assert!(!without_stage.database);
    let mut with = stage_request(&[], "{}");
    with.extensions_mut()
        .insert(state_for(&config_with_database(true)));
    let Ok(StageInput::AuthEndUserIdentity(with_stage)) = stage_input(&with, Some(&context), None)
    else {
        panic!("the authenticated request builds its stage input");
    };
    assert!(with_stage.database);
}

#[test]
fn stage_uses_the_server_tracker() {
    let tasks = TaskTracker::new();
    let master_key = SecretString::from("secret-master-key".to_owned());
    let context = master_context(&master_key);
    let mut request = stage_request(&[], "{}");
    request.extensions_mut().insert(tasks.clone());
    let Ok(StageInput::AuthEndUserIdentity(stage)) = stage_input(&request, Some(&context), None)
    else {
        panic!("the authenticated request builds its stage input");
    };
    let inherited = stage.tasks.expect("the server tracker reaches the stage");
    assert!(TaskTracker::ptr_eq(&tasks, &inherited));
    let pending = inherited.token();
    assert_eq!(tasks.len(), 1);
    tasks.close();
    assert!(inherited.is_closed());
    drop(pending);
    assert!(tasks.is_empty());
}

#[test]
fn the_rule_debug_names_the_wired_resolver() {
    let cache = ScriptedCache::new(false);
    let (identity, _existence) = resolver(true, &cache);
    let rule = EndUserIdentityRule {
        resolver: Some(identity),
    };
    let debug = format!("{rule:?}");
    assert!(debug.contains("EndUserIdResolver"), "{debug}");
    assert!(!debug.contains("ScriptedCache"), "{debug}");
}

#[test]
fn stage_debug_hides_server_state() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let context = master_context(&master_key);
    let mut request = stage_request(&[], "{}");
    request
        .extensions_mut()
        .insert(state_for(&config_with_database(false)));
    request.extensions_mut().insert(TaskTracker::new());
    let Ok(StageInput::AuthEndUserIdentity(mut stage)) =
        stage_input(&request, Some(&context), None)
    else {
        panic!("the authenticated request builds its stage input");
    };
    let wired = format!("{stage:?}");
    assert!(wired.contains("EndUserIdentityStageInput"), "{wired}");
    assert!(wired.contains("state: true"), "{wired}");
    assert!(wired.contains("tasks: true"), "{wired}");
    assert!(!wired.contains("ServerState"), "{wired}");
    stage.state = None;
    stage.tasks = None;
    let detached = format!("{stage:?}");
    assert!(detached.contains("state: false"), "{detached}");
    assert!(detached.contains("tasks: false"), "{detached}");
}

#[test]
fn request_id_sources() {
    let config = config_with_database(true);
    let standard = extracted(
        &config,
        &[
            ("x-litellm-end-user-id", "from-standard"),
            ("x-litellm-customer-id", "from-customer"),
        ],
        r#"{"user": "from-body"}"#,
    );
    assert_eq!(standard.as_deref(), Some("from-customer"));
    let body = extracted(&config, &[], r#"{"user": "from-body"}"#);
    assert_eq!(body.as_deref(), Some("from-body"));
    let number = extracted(&config, &[], r#"{"user": 42}"#);
    assert_eq!(number.as_deref(), Some("42"));
    let absent = extracted(&config, &[], "{}");
    assert!(absent.is_none());
}

#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Option<String>>>);

async fn mark_called(
    Extension(seen): Extension<Seen>,
    Extension(context): Extension<AuthContext>,
) -> &'static str {
    *seen.0.lock().expect("the seen end user") =
        context.principal.end_user.map(|end_user| end_user.id);
    "reached"
}

async fn run_request(
    config: &Config,
    identity_rule: Option<Arc<EndUserIdentityRule>>,
    end_user_rule: Option<Arc<EndUserRule>>,
    end_user_id: &str,
) -> (StatusCode, String, Option<String>) {
    let seen = Seen::default();
    let tasks = TaskTracker::new();
    let mut request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header("x-litellm-end-user-id", end_user_id)
        .body(Body::empty())
        .expect("the request builds");
    request.extensions_mut().insert(seen.clone());
    let mut application = routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(tasks.clone()));
    if let Some(rule) = identity_rule {
        application = application.layer(Extension(rule));
    }
    if let Some(rule) = end_user_rule {
        application = application.layer(Extension(rule));
    }
    let response = application
        .oneshot(request)
        .await
        .expect("the request reaches the router");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the response body is UTF-8");
    let id = seen.0.lock().expect("the seen end user").clone();
    tasks.close();
    tasks.wait().await;
    (status, text, id)
}

#[tokio::test]
async fn failure_stops_downstream() {
    let cache = ScriptedCache::new(true);
    let (identity, _existence) = resolver(true, &cache);
    let rule = Arc::new(EndUserIdentityRule {
        resolver: Some(identity),
    });
    let (status, body, seen) =
        run_request(&config_with_database(true), Some(rule), None, "ghost").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body.contains("\"type\":\"internal_server_error\""),
        "{body}"
    );
    assert!(
        body.contains("\"message\":\"Internal server error\""),
        "{body}"
    );
    assert!(body.contains("\"param\":null"), "{body}");
    assert!(body.contains("\"code\":\"500\""), "{body}");
    assert!(seen.is_none(), "the downstream handler was not called");
    assert_eq!(cache.reads(), vec!["end_user_validation:ghost".to_owned()]);
}

#[tokio::test]
async fn resolved_id_reaches_the_downstream_handler() {
    let cache = ScriptedCache::new(false);
    let (identity, existence) = resolver(true, &cache);
    let rule = Arc::new(EndUserIdentityRule {
        resolver: Some(identity),
    });
    let (status, _body, seen) =
        run_request(&config_with_database(true), Some(rule), None, "alice").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seen.as_deref(), Some("alice"));
    assert_eq!(existence.calls(), vec!["alice".to_owned()]);
    assert_eq!(cache.reads(), vec!["end_user_validation:alice".to_owned()]);
}

#[tokio::test]
async fn pass_through_without_a_database() {
    let cache = ScriptedCache::new(true);
    let (identity, existence) = resolver(true, &cache);
    let master_key = SecretString::from("secret-master-key".to_owned());
    let mut without_database = EndUserIdentityStageInput {
        context: master_context(&master_key),
        raw_end_user_id: Some("alice".to_owned()),
        rule: Arc::new(EndUserIdentityRule {
            resolver: Some(identity),
        }),
        database: false,
        state: None,
        tasks: None,
    };
    prefetch(&mut without_database)
        .await
        .expect("the database gate passes the id through");
    assert_eq!(end_user_id(&without_database.context), Some("alice"));
    assert!(cache.reads().is_empty());
    assert!(existence.calls().is_empty());
    let mut without_resolver = EndUserIdentityStageInput {
        context: master_context(&master_key),
        raw_end_user_id: Some("bob".to_owned()),
        rule: Arc::new(EndUserIdentityRule::default()),
        database: true,
        state: None,
        tasks: None,
    };
    prefetch(&mut without_resolver)
        .await
        .expect("an absent resolver passes the id through");
    assert_eq!(end_user_id(&without_resolver.context), Some("bob"));
    without_resolver.raw_end_user_id = None;
    prefetch(&mut without_resolver)
        .await
        .expect("no id resolves to none");
    assert_eq!(end_user_id(&without_resolver.context), None);
}

fn end_user_id(context: &AuthContext) -> Option<&str> {
    context
        .principal
        .end_user
        .as_ref()
        .map(|end_user| end_user.id.as_str())
}

// the production resolver the state builds, reached through the real server constructor with no test rule
#[tokio::test]
async fn production_validation_drops_an_unknown_id() {
    let (status, body, seen) =
        run_request(&config_with_database(true), None, None, "ghost-b880").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(seen.is_none(), "the unknown id never reaches the handler");

    // the same id without a database passes through, so the drop above is the validation
    let (_, _, passthrough) =
        run_request(&config_with_master_key(), None, None, "ghost-b880").await;
    assert_eq!(passthrough.as_deref(), Some("ghost-b880"));
}

fn inference_request() -> Request<Body> {
    let mut request = stage_request(
        &[
            ("authorization", "Bearer secret-master-key"),
            ("x-litellm-end-user-id", "ghost"),
        ],
        r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hello"}]}"#,
    );
    *request.uri_mut() = "/v1/chat/completions".parse().unwrap();
    request
}

#[tokio::test]
async fn application_tracks_validation() {
    let config = config_with_database(true);
    let tasks = TaskTracker::new();
    let chat = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
    let lookup = Arc::new(ScriptedEndUserLookup::default());
    let app = server_application(&config, chat, Arc::new(AtomicBool::new(false)), &tasks)
        .unwrap()
        .layer(Extension(end_user_rule(&lookup, None)));
    let mut known = inference_request();
    let (identity, _existence) = resolver(true, &ScriptedCache::new(false));
    known.extensions_mut().insert(Arc::new(EndUserIdentityRule {
        resolver: Some(identity),
    }));
    app.clone().oneshot(known).await.unwrap();
    assert_eq!(lookup.lookups(), vec!["ghost".to_owned()]);
    lookup.lookups.lock().unwrap().clear();
    app.oneshot(inference_request()).await.unwrap();
    assert!(
        lookup.lookups().is_empty(),
        "the production resolver drops the unknown ID"
    );
    tasks.close();
    tasks.wait().await;
}

// a database alone does not open the lookup: LiteLLM's validation flag must be on too
#[tokio::test]
async fn the_production_resolver_needs_the_validation_flag() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let mut request = stage_request(&[("x-litellm-end-user-id", "alice")], "{}");
    request
        .extensions_mut()
        .insert(state_for(&config_with_database(false)));
    let tasks = TaskTracker::new();
    request.extensions_mut().insert(tasks.clone());
    let Ok(StageInput::AuthEndUserIdentity(stage)) =
        stage_input(&request, Some(&master_context(&master_key)), None)
    else {
        panic!("the authenticated request builds its stage input");
    };
    assert!(
        installed_resolver(&stage).await.is_none(),
        "the resolver is built only when validation is on"
    );
    tasks.close();
    tasks.wait().await;
}

fn key_with_end_user_budget(budget_id: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(&format!(
        r#"{{"api_key": "sk-test", "metadata": {{"end_user_budget_id": "{budget_id}"}}}}"#
    ))
    .expect("the key fixture is JSON");
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture constructs")
}

fn keyed_context(master_key: &SecretString, budget_id: &str) -> AuthContext {
    let mut context = master_context(master_key);
    context.principal.source_key = SourceKey(Some(key_with_end_user_budget(budget_id)));
    context
}

#[tokio::test]
async fn key_budget_id_keeps_an_unknown_id() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let cache = ScriptedCache::new(false);
    let (kept_resolver, _kept_existence) = resolver(false, &cache);
    let mut kept = EndUserIdentityStageInput {
        context: keyed_context(&master_key, "budget-key"),
        raw_end_user_id: Some("ghost".to_owned()),
        rule: Arc::new(EndUserIdentityRule {
            resolver: Some(kept_resolver),
        }),
        database: true,
        state: None,
        tasks: None,
    };
    prefetch(&mut kept).await.expect("the end-user validation");
    assert_eq!(end_user_id(&kept.context), Some("ghost"));

    // the same unknown id without a key budget is dropped
    let (dropped_resolver, _dropped_existence) = resolver(false, &cache);
    let mut dropped = EndUserIdentityStageInput {
        context: master_context(&master_key),
        raw_end_user_id: Some("ghost".to_owned()),
        rule: Arc::new(EndUserIdentityRule {
            resolver: Some(dropped_resolver),
        }),
        database: true,
        state: None,
        tasks: None,
    };
    prefetch(&mut dropped)
        .await
        .expect("the end-user validation");
    assert_eq!(end_user_id(&dropped.context), None);
}

#[derive(Debug, Default)]
struct ScriptedEndUserLookup {
    lookups: Mutex<Vec<String>>,
    checks: Mutex<Vec<String>>,
}

impl ScriptedEndUserLookup {
    fn lookups(&self) -> Vec<String> {
        self.lookups.lock().expect("the lookup calls").clone()
    }

    fn checks(&self) -> Vec<String> {
        self.checks.lock().expect("the budget checks").clone()
    }
}

fn end_user_row() -> LiteLLMEndUserTable {
    LiteLLMEndUserTable {
        user_id: "alice".to_owned(),
        blocked: PydanticBool(false),
        alias: None,
        spend: Decimal::from(0_i64),
        allowed_model_region: None,
        default_model: None,
        budget_id: None,
        litellm_budget_table: None,
        object_permission_id: None,
        object_permission: None,
    }
}

#[async_trait]
impl EndUserLookup for ScriptedEndUserLookup {
    async fn end_user_object(
        &self,
        end_user_id: &str,
        _key_end_user_budget_id: Option<&str>,
        _token_end_user_max_budget: Option<Decimal>,
    ) -> Result<Option<LiteLLMEndUserTable>, EndUserFailure> {
        self.lookups
            .lock()
            .expect("the lookup calls")
            .push(end_user_id.to_owned());
        Ok(Some(end_user_row()))
    }

    async fn check_budget(
        &self,
        end_user: &LiteLLMEndUserTable,
        route: &str,
    ) -> Result<(), EndUserFailure> {
        self.checks
            .lock()
            .expect("the budget checks")
            .push(format!("{} {route}", end_user.user_id));
        Ok(())
    }
}

fn end_user_rule(
    lookup: &Arc<ScriptedEndUserLookup>,
    end_user_id: Option<&str>,
) -> Arc<EndUserRule> {
    Arc::new(EndUserRule {
        end_user_id: end_user_id.map(str::to_owned),
        key_end_user_budget_id: Some("budget-key".to_owned()),
        token_end_user_max_budget: None,
        lookup: Some(Arc::<ScriptedEndUserLookup>::clone(lookup)),
    })
}

#[tokio::test]
async fn the_end_user_budget_stage_reads_the_validated_id() {
    let cache = ScriptedCache::new(false);
    let (identity, _existence) = resolver(true, &cache);
    let lookup = Arc::new(ScriptedEndUserLookup::default());
    let (status, body, seen) = run_request(
        &config_with_database(true),
        Some(Arc::new(EndUserIdentityRule {
            resolver: Some(identity),
        })),
        Some(end_user_rule(&lookup, None)),
        "alice",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(seen.as_deref(), Some("alice"));
    assert_eq!(lookup.lookups(), vec!["alice".to_owned()]);
    assert_eq!(lookup.checks(), vec!["alice /v1/files".to_owned()]);
}

#[tokio::test]
async fn a_dropped_id_clears_a_preseeded_end_user_lookup() {
    let cache = ScriptedCache::new(false);
    let (identity, _existence) = resolver(false, &cache);
    let lookup = Arc::new(ScriptedEndUserLookup::default());
    let (status, body, seen) = run_request(
        &config_with_database(true),
        Some(Arc::new(EndUserIdentityRule {
            resolver: Some(identity),
        })),
        Some(end_user_rule(&lookup, Some("stale"))),
        "ghost",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(seen.is_none());
    assert!(
        lookup.lookups().is_empty(),
        "the dropped id never reaches the budget lookup"
    );
    assert!(lookup.checks().is_empty());
}

#[tokio::test]
async fn disabled_validation_without_id_clears_a_preseeded_budget_id() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let lookup = Arc::new(ScriptedEndUserLookup::default());
    let mut request = stage_request(&[], "{}");
    request
        .extensions_mut()
        .insert(state_for(&config_with_database(false)));
    request
        .extensions_mut()
        .insert(end_user_rule(&lookup, Some("unrelated-user")));
    let Ok(stage) = auth_end_users::stage_input(&request, Some(&master_context(&master_key)))
    else {
        panic!("the budget stage input is built");
    };
    assert_eq!(stage.rule.end_user_id.as_deref(), Some("unrelated-user"));
    let Ok(StageInput::AuthEndUsers(stage)) = prefetch_budget(Box::new(stage)).await else {
        panic!("an absent ID skips the end-user budget lookup");
    };
    assert!(stage.rule.end_user_id.is_none());
    assert!(lookup.lookups().is_empty());
    assert!(lookup.checks().is_empty());
}
