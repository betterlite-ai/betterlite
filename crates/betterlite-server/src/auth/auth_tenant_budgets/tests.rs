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
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_tag_budgets::cache::{CachedValue, TagBudgetCache};
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_auth::auth_tag_budgets::rows::{BudgetRows, TagBudgetRowsFailure};
use betterlite_auth::current_spend::{BUDGET_UNVERIFIABLE_DETAIL, SpendQuery};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::json;
use tower::ServiceExt as _;

use super::{TenantBudgetRule, TenantBudgetStageInput, prefetch, stage_input};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";
const MASTER_KEY: &str = "secret-master-key";

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

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthTenantBudgets));
    let tenant = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::AuthTenantBudgets);
    let team = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::TeamBudgetAdmission);
    assert!(
        tenant > team,
        "the tenant checks run after the team admission: {INSTALLED_STAGES:?}"
    );
    // LiteLLM runs the checks only when a database is configured
    let without = state_for(&config_with_master_key());
    assert!(!without.auth_tenant_budgets.database);
    let with = state_for(&config_with_database());
    assert!(with.auth_tenant_budgets.database);

    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let stage = TenantBudgetStageInput {
        context: master_context(&master_key),
        rule: Arc::new(TenantBudgetRule::default()),
        tags: Vec::new(),
        database: true,
    };
    let input = StageInput::AuthTenantBudgets(Box::new(stage));
    let Ok(forwarded) = dispatch(AuthBackend::AuthTenantBudgets, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production tenant budget stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthTenantBudgets, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));
}

struct ScriptedReader {
    spends: Mutex<Vec<Decimal>>,
}

impl ScriptedReader {
    fn new(spends: Vec<Decimal>) -> Arc<Self> {
        Arc::new(Self {
            spends: Mutex::new(spends),
        })
    }
}

#[async_trait]
impl SpendReader for ScriptedReader {
    async fn read_spend(&self, _query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        let mut spends = self.spends.lock().expect("the spends lock");
        assert!(!spends.is_empty(), "the reader ran out of scripted spends");
        Ok(spends.remove(0))
    }
}

struct TagWorld {
    tags: BTreeMap<String, TagTable>,
}

impl TagWorld {
    fn new(tags: Vec<TagTable>) -> Arc<Self> {
        Arc::new(Self {
            tags: tags
                .into_iter()
                .map(|row| (row.tag_name.clone(), row))
                .collect(),
        })
    }
}

#[async_trait]
impl TagBudgetCache for TagWorld {
    async fn read_tag(&self, _key: &str) -> Result<Option<TagTable>, PayloadError> {
        Ok(None)
    }

    async fn read_group(&self, _key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        Ok(None)
    }

    async fn read_registry(&self, _key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _value: CachedValue<'_>,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

#[async_trait]
impl BudgetRows<TagTable> for TagWorld {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(self
            .tags
            .values()
            .filter(|tag| names.contains(&tag.tag_name))
            .cloned()
            .collect())
    }

    async fn registry_rows(&self, _take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Ok(self.tags.values().cloned().collect())
    }
}

fn tag(name: &str, spend: i64, max_budget: i64) -> TagTable {
    serde_json::from_value(json!({
        "tag_name": name,
        "models": [],
        "spend": spend,
        "litellm_budget_table": {"max_budget": max_budget},
    }))
    .expect("the tag row decodes")
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

fn request_to(reached: &Arc<AtomicBool>, body: &'static str) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(reached));
    request
}

fn application(config: &Config, rule: Arc<TenantBudgetRule>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule))
}

async fn run_request(
    config: &Config,
    rule: Arc<TenantBudgetRule>,
    body: &'static str,
) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(config, rule)
        .oneshot(request_to(&reached, body))
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
    let world = TagWorld::new(vec![tag("over", 0, 10)]);
    let reader = ScriptedReader::new(vec![Decimal::from(20_i64)]);
    let cache: Arc<dyn TagBudgetCache> = Arc::clone(&world);
    let rows: Arc<dyn BudgetRows<TagTable>> = world;
    let rule = Arc::new(TenantBudgetRule {
        reader: Some(reader),
        cache: Some(cache),
        tags: Some(rows),
        ..TenantBudgetRule::default()
    });
    let (status, body, reached) =
        run_request(&config_with_database(), rule, r#"{"tags":["over"]}"#).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("\"type\":\"budget_exceeded\""), "{body}");
    assert!(
        body.contains("Budget has been exceeded! Tag=over Current cost: 20, Max budget: 10"),
        "{body}"
    );
    assert!(body.contains("\"param\":null"), "{body}");
    assert!(body.contains("\"code\":\"422\""), "{body}");
    assert!(!reached);

    // without a database the tag check never reads, so the same request reaches the handler
    let world = TagWorld::new(vec![tag("over", 0, 10)]);
    let reader = ScriptedReader::new(vec![Decimal::from(20_i64)]);
    let cache: Arc<dyn TagBudgetCache> = Arc::clone(&world);
    let rows: Arc<dyn BudgetRows<TagTable>> = world;
    let rule = Arc::new(TenantBudgetRule {
        reader: Some(reader),
        cache: Some(cache),
        tags: Some(rows),
        ..TenantBudgetRule::default()
    });
    let (status, _body, reached) =
        run_request(&config_with_master_key(), rule, r#"{"tags":["over"]}"#).await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
}

// a key the tenant stage can hand on, with fields the stored copy keeps
fn checked_key() -> UserAPIKeyAuth {
    let JsonData::Object(map) =
        serde_json::from_str(r#"{"token": "sk-checked", "user_id": "user-1"}"#)
            .expect("the key fixture is JSON")
    else {
        panic!("the key fields must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).expect("the key fixture constructs")
}

#[test]
fn debug_names_the_slots_and_never_the_token() {
    let key = checked_key();
    let rule = TenantBudgetRule {
        auth: Some(key.clone()),
        applied: Mutex::new(Some(key)),
        ..TenantBudgetRule::default()
    };
    let rendered = format!("{rule:?}");
    assert!(rendered.contains("has_auth: true"), "{rendered}");
    assert!(rendered.contains("has_applied: true"), "{rendered}");
    assert!(!rendered.contains("sk-checked"), "{rendered}");
}

#[tokio::test]
async fn prefetch_records_the_checked_key() {
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let rule = Arc::new(TenantBudgetRule {
        auth: Some(checked_key()),
        ..TenantBudgetRule::default()
    });
    let stage = TenantBudgetStageInput {
        context: master_context(&master_key),
        rule: Arc::clone(&rule),
        tags: Vec::new(),
        database: false,
    };
    prefetch(&stage).await.expect("an unconfigured rule admits");
    let applied = rule.applied().expect("the stage records the checked key");
    let fields = serde_json::to_value(&applied).expect("the checked key serializes");
    assert_eq!(fields["user_id"], json!("user-1"));
    assert_eq!(fields["token"], json!("sk-checked"));
}

// a cache whose tag probe fails, which is the only batch failure that answers 401
struct BrokenTags;

#[async_trait]
impl TagBudgetCache for BrokenTags {
    async fn read_tag(&self, _key: &str) -> Result<Option<TagTable>, PayloadError> {
        Err(PayloadError::Json(serde_json::Error::io(
            std::io::Error::other("the cache read failed"),
        )))
    }

    async fn read_group(&self, _key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        Ok(None)
    }

    async fn read_registry(&self, _key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _value: CachedValue<'_>,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }
}

#[tokio::test]
async fn tag_lookup_failure_answers_unauthorized() {
    let world = TagWorld::new(Vec::new());
    let rows: Arc<dyn BudgetRows<TagTable>> = world;
    let cache: Arc<dyn TagBudgetCache> = Arc::new(BrokenTags);
    let rule = Arc::new(TenantBudgetRule {
        cache: Some(cache),
        tags: Some(rows),
        ..TenantBudgetRule::default()
    });
    let (status, body, reached) =
        run_request(&config_with_database(), rule, r#"{"tags":["broken"]}"#).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(
        body.contains("Authentication Error, the cached tag payload could not be read"),
        "{body}"
    );
    assert!(!reached);
}

#[tokio::test]
async fn missing_reader_is_unavailable() {
    let world = TagWorld::new(vec![tag("t", 0, 10)]);
    let cache: Arc<dyn TagBudgetCache> = Arc::clone(&world);
    let rows: Arc<dyn BudgetRows<TagTable>> = world;
    let rule = Arc::new(TenantBudgetRule {
        cache: Some(cache),
        tags: Some(rows),
        ..TenantBudgetRule::default()
    });
    let (status, body, reached) =
        run_request(&config_with_database(), rule, r#"{"tags":["t"]}"#).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains(BUDGET_UNVERIFIABLE_DETAIL), "{body}");
    assert!(!reached);
}

fn tenant_stage(context: &AuthContext, body: &'static str) -> TenantBudgetStageInput {
    let request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .body(Body::empty())
        .expect("the request builds");
    let bytes = Bytes::from_static(body.as_bytes());
    stage_input(&request, Some(context), Some(&bytes)).expect("the stage input builds")
}

#[test]
fn metadata_and_body_tags_string_only_ordered_duplicates() {
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let context = master_context(&master_key);

    // the metadata object's tags come first, then the body's, with nonstrings dropped and duplicates kept
    let object = tenant_stage(
        &context,
        r#"{"metadata":{"tags":["m1",7,"m1",null]},"tags":["top","m1",true]}"#,
    );
    assert_eq!(object.tags, vec!["m1", "m1", "top", "m1"]);

    // a metadata field that arrived as a JSON string reads the same way
    let encoded = tenant_stage(
        &context,
        r#"{"metadata":"{\"tags\":[\"s1\",\"s1\"]}","tags":["top"]}"#,
    );
    assert_eq!(encoded.tags, vec!["s1", "s1", "top"]);
}

#[test]
fn litellm_metadata_selects_the_metadata_field() {
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let context = master_context(&master_key);

    // litellm_metadata is the metadata field LiteLLM selects when the body carries both
    let selected = tenant_stage(
        &context,
        r#"{"litellm_metadata":{"tags":["chosen"]},"metadata":{"tags":["ignored"]}}"#,
    );
    assert_eq!(selected.tags, vec!["chosen"]);

    // a tags value that is not a list carries nothing
    let scalar = tenant_stage(&context, r#"{"tags":"single"}"#);
    assert!(scalar.tags.is_empty());
}

#[test]
fn bodyless_requests_carry_no_tags_and_require_a_caller() {
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let context = master_context(&master_key);

    let no_body = stage_input(
        &Request::builder()
            .method("POST")
            .uri(MODEL_ROUTE)
            .body(Body::empty())
            .expect("the request builds"),
        Some(&context),
        None,
    )
    .expect("the stage input builds");
    assert!(no_body.tags.is_empty());
    assert!(!no_body.database);
    stage_input(
        &Request::builder()
            .method("POST")
            .uri(MODEL_ROUTE)
            .body(Body::empty())
            .expect("the request builds"),
        None,
        None,
    )
    .expect_err("the stage needs an authenticated caller");
}
