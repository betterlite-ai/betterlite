#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_tag_budgets::TagBudgetError;
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::managed_resource_rows::tag::TagTable;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{TagBudgetLookup, TagBudgetRule, TagBudgetStageInput, prefetch};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Resolved,
    TagFailure,
    GroupFailure,
}

struct ScriptedLookup {
    answer: Answer,
    tag_calls: Mutex<Vec<Vec<String>>>,
    group_calls: Mutex<Vec<Vec<String>>>,
}

impl ScriptedLookup {
    fn new(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer,
            tag_calls: Mutex::new(Vec::new()),
            group_calls: Mutex::new(Vec::new()),
        })
    }

    fn tag_calls(&self) -> Vec<Vec<String>> {
        self.tag_calls.lock().expect("the tag calls lock").clone()
    }

    fn group_calls(&self) -> Vec<Vec<String>> {
        self.group_calls
            .lock()
            .expect("the group calls lock")
            .clone()
    }
}

fn payload_error() -> PayloadError {
    PayloadError::Json(serde_json::from_str::<i64>("not a number").expect_err("the invalid json"))
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[async_trait]
impl TagBudgetLookup for ScriptedLookup {
    async fn tag_objects(
        &self,
        values: &[String],
        database: bool,
    ) -> Result<BTreeMap<String, TagTable>, TagBudgetError> {
        assert!(database, "the absent database never reaches the lookup");
        self.tag_calls
            .lock()
            .expect("the tag calls lock")
            .push(values.to_vec());
        match self.answer {
            Answer::TagFailure => Err(TagBudgetError::TagPayload(Box::new(payload_error()))),
            Answer::Resolved | Answer::GroupFailure => Ok(BTreeMap::new()),
        }
    }

    async fn group_budgets(
        &self,
        values: &[String],
        database: bool,
    ) -> Result<BTreeMap<String, ModelAccessGroupBudget>, TagBudgetError> {
        assert!(database, "the absent database never reaches the lookup");
        self.group_calls
            .lock()
            .expect("the group calls lock")
            .push(values.to_vec());
        match self.answer {
            Answer::GroupFailure => Err(TagBudgetError::GroupPayload(Box::new(payload_error()))),
            Answer::Resolved | Answer::TagFailure => Ok(BTreeMap::new()),
        }
    }
}

fn rule(lookup: Arc<ScriptedLookup>, tags: &[&str], groups: &[&str]) -> Arc<TagBudgetRule> {
    Arc::new(TagBudgetRule {
        tag_names: names(tags),
        model_access_group_names: names(groups),
        lookup: Some(lookup),
    })
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
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthTagBudgets));
    // LiteLLM runs both batch reads only when a database is configured
    let without = state_for(&config_with_master_key());
    assert!(!without.auth_tag_budgets.database);
    let with = state_for(&config_with_database());
    assert!(with.auth_tag_budgets.database);

    let master_key = SecretString::from("secret-master-key".to_owned());
    let lookup = ScriptedLookup::new(Answer::Resolved);
    let stage = TagBudgetStageInput {
        context: master_context(&master_key),
        rule: rule(Arc::clone(&lookup), &["tag-one"], &["group-one"]),
        database: true,
    };
    let input = StageInput::AuthTagBudgets(Box::new(stage));
    let Ok(forwarded) = dispatch(AuthBackend::AuthTagBudgets, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production tag budget stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthTagBudgets, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));
}

#[tokio::test]
async fn prefetch_resolves_every_name() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let lookup = ScriptedLookup::new(Answer::Resolved);
    let stage = TagBudgetStageInput {
        context: master_context(&master_key),
        rule: rule(Arc::clone(&lookup), &["tag-one", "tag-two"], &["group-one"]),
        database: true,
    };
    prefetch(&stage).await.expect("the batch reads resolve");
    assert_eq!(lookup.tag_calls(), vec![names(&["tag-one", "tag-two"])]);
    assert_eq!(lookup.group_calls(), vec![names(&["group-one"])]);
}

#[tokio::test]
async fn absent_database_skips_the_lookup() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let lookup = ScriptedLookup::new(Answer::TagFailure);
    let stage = TagBudgetStageInput {
        context: master_context(&master_key),
        rule: rule(Arc::clone(&lookup), &["tag-one"], &["group-one"]),
        database: false,
    };
    prefetch(&stage)
        .await
        .expect("the absent database fails open");
    assert!(lookup.tag_calls().is_empty());
    assert!(lookup.group_calls().is_empty());
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

fn application(config: &Config, rule: Arc<TagBudgetRule>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule))
}

async fn run_request(config: &Config, rule: Arc<TagBudgetRule>) -> (StatusCode, String, bool) {
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
    let lookup = ScriptedLookup::new(Answer::TagFailure);
    let (status, body, reached) = run_request(
        &config_with_database(),
        rule(Arc::clone(&lookup), &["tag-one"], &[]),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(
        body.contains("Authentication Error, the cached tag payload could not be read"),
        "{body}"
    );
    assert!(!reached);
    assert_eq!(lookup.tag_calls(), vec![names(&["tag-one"])]);

    let group_lookup = ScriptedLookup::new(Answer::GroupFailure);
    let (group_status, group_body, group_reached) = run_request(
        &config_with_database(),
        rule(Arc::clone(&group_lookup), &[], &["group-one"]),
    )
    .await;
    assert_eq!(group_status, StatusCode::UNAUTHORIZED);
    assert!(
        group_body.contains("the cached model access group budget payload could not be read"),
        "{group_body}"
    );
    assert!(!group_reached);
    assert_eq!(group_lookup.group_calls(), vec![names(&["group-one"])]);
}

#[tokio::test]
async fn resolved_budgets_reach_the_handler() {
    let lookup = ScriptedLookup::new(Answer::Resolved);
    let (status, _body, reached) = run_request(
        &config_with_database(),
        rule(Arc::clone(&lookup), &["tag-one"], &["group-one"]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
    assert_eq!(lookup.tag_calls(), vec![names(&["tag-one"])]);
    assert_eq!(lookup.group_calls(), vec![names(&["group-one"])]);

    // without a database the same rule resolves nothing and the request still reaches the handler
    let skipped = ScriptedLookup::new(Answer::TagFailure);
    let (skip_status, _skip_body, skip_reached) = run_request(
        &config_with_master_key(),
        rule(Arc::clone(&skipped), &["tag-one"], &[]),
    )
    .await;
    assert_eq!(skip_status, StatusCode::OK);
    assert!(skip_reached);
    assert!(skipped.tag_calls().is_empty());
}
