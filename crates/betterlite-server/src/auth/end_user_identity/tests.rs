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
use betterlite_auth::end_user_identity::cache::ValidationCache;
use betterlite_auth::end_user_identity::existence::{
    EndUserExistence, ExistenceError, UserExistence, UserProbe,
};
use betterlite_auth::end_user_identity::resolve::{DatabaseLookups, EndUserIdResolver};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{EndUserIdentityRule, EndUserIdentityStageInput, prefetch, stage_input};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

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

fn config_with_database() -> Config {
    let mut lookup = BTreeMap::<String, String>::new();
    lookup.insert(
        "DATABASE_URL".to_owned(),
        "postgresql://user:password@localhost:5432/betterlite".to_owned(),
    );
    ConfigLoader { lookup }
        .load(
            "general_settings:\n  master_key: secret-master-key\nlitellm_settings:\n  validate_end_user_id_in_db: true\n",
        )
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
    let with = state_for(&config_with_database());
    assert!(with.end_user_identity.database);
    assert!(with.end_user_identity.validate_end_user_id_in_db);

    let master_key = SecretString::from("secret-master-key".to_owned());
    let stage = EndUserIdentityStageInput {
        context: master_context(&master_key),
        raw_end_user_id: Some("alice".to_owned()),
        rule: Arc::new(EndUserIdentityRule::default()),
        database: true,
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
    let Ok(stage) = stage_input(&request, Some(&context), Some(&bytes)) else {
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
    let Ok(without_stage) = stage_input(&without, Some(&context), None) else {
        panic!("the authenticated request builds its stage input");
    };
    assert!(!without_stage.database);
    let mut with = stage_request(&[], "{}");
    with.extensions_mut()
        .insert(state_for(&config_with_database()));
    let Ok(with_stage) = stage_input(&with, Some(&context), None) else {
        panic!("the authenticated request builds its stage input");
    };
    assert!(with_stage.database);
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
fn request_id_sources() {
    let config = config_with_database();
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
    rule: Arc<EndUserIdentityRule>,
    end_user_id: &str,
) -> (StatusCode, String, Option<String>) {
    let seen = Seen::default();
    let mut request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header("x-litellm-end-user-id", end_user_id)
        .body(Body::empty())
        .expect("the request builds");
    request.extensions_mut().insert(seen.clone());
    let application = routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule));
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
    (status, text, id)
}

#[tokio::test]
async fn failure_stops_downstream() {
    let cache = ScriptedCache::new(true);
    let (identity, _existence) = resolver(true, &cache);
    let rule = Arc::new(EndUserIdentityRule {
        resolver: Some(identity),
    });
    let (status, body, seen) = run_request(&config_with_database(), rule, "ghost").await;
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
    let (status, _body, seen) = run_request(&config_with_database(), rule, "alice").await;
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
    };
    prefetch(&mut without_resolver)
        .await
        .expect("an absent resolver passes the id through");
    assert_eq!(end_user_id(&without_resolver.context), Some("bob"));
    let mut no_id = EndUserIdentityStageInput {
        context: master_context(&master_key),
        raw_end_user_id: None,
        rule: Arc::new(EndUserIdentityRule::default()),
        database: true,
    };
    prefetch(&mut no_id).await.expect("no id resolves to none");
    assert_eq!(end_user_id(&no_id.context), None);
}

fn end_user_id(context: &AuthContext) -> Option<&str> {
    context
        .principal
        .end_user
        .as_ref()
        .map(|end_user| end_user.id.as_str())
}
