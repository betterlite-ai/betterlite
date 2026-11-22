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
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::identity_rows::PydanticBool;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_error::error::details::BudgetExceededDetails;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{EndUserFailure, EndUserLookup, EndUserRule, EndUserStageInput, prefetch};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Resolved,
    Missing,
    Budget,
    Failure,
}

#[derive(Debug)]
struct ScriptedLookup {
    answer: Answer,
    lookups: Mutex<Vec<String>>,
    checks: Mutex<Vec<String>>,
}

impl ScriptedLookup {
    fn new(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer,
            lookups: Mutex::new(Vec::new()),
            checks: Mutex::new(Vec::new()),
        })
    }

    fn lookups(&self) -> Vec<String> {
        self.lookups.lock().expect("the lookup calls").clone()
    }

    fn checks(&self) -> Vec<String> {
        self.checks.lock().expect("the budget checks").clone()
    }
}

fn row() -> LiteLLMEndUserTable {
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

fn budget_details() -> BudgetExceededDetails {
    BudgetExceededDetails {
        current_cost: Decimal::from(150_i64),
        max_budget: Decimal::from(100_i64),
        message: Some(
            "ExceededBudget: End User=alice over budget. Spend=150, Budget=100".to_owned(),
        ),
        provider: String::new(),
        entity_type: Some("end_user".to_owned()),
        entity_id: Some("alice".to_owned()),
        status_code: 422,
    }
}

#[async_trait]
impl EndUserLookup for ScriptedLookup {
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
        match self.answer {
            Answer::Resolved | Answer::Budget => Ok(Some(row())),
            Answer::Missing => Ok(None),
            Answer::Failure => Err(EndUserFailure::Failure(
                "the end-user lookup failed".to_owned(),
            )),
        }
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
        if self.answer == Answer::Budget {
            return Err(EndUserFailure::Budget(budget_details()));
        }
        Ok(())
    }
}

fn rule(lookup: Arc<ScriptedLookup>, end_user_id: Option<&str>) -> Arc<EndUserRule> {
    Arc::new(EndUserRule {
        end_user_id: end_user_id.map(str::to_owned),
        key_end_user_budget_id: Some("key-budget".to_owned()),
        token_end_user_max_budget: None,
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
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthEndUsers));
    // LiteLLM's get_end_user_object needs a database, and the proxy default comes from litellm_settings
    let without = state_for(&config_with_master_key());
    assert!(!without.auth_end_users.database);
    assert_eq!(without.auth_end_users.max_end_user_budget_id, None);
    let mut lookup = BTreeMap::<String, String>::new();
    lookup.insert(
        "DATABASE_URL".to_owned(),
        "postgresql://user:password@localhost:5432/betterlite".to_owned(),
    );
    let configured = ConfigLoader { lookup }
        .load(
            "general_settings:\n  master_key: secret-master-key\nlitellm_settings:\n  max_end_user_budget_id: budget-global\n",
        )
        .expect("the config loads");
    let with = state_for(&configured);
    assert!(with.auth_end_users.database);
    assert_eq!(
        with.auth_end_users.max_end_user_budget_id.as_deref(),
        Some("budget-global")
    );

    let master_key = SecretString::from("secret-master-key".to_owned());
    let stage = EndUserStageInput {
        context: master_context(&master_key),
        rule: rule(ScriptedLookup::new(Answer::Resolved), Some("alice")),
        route: MODEL_ROUTE.to_owned(),
        database: true,
    };
    let input = StageInput::AuthEndUsers(Box::new(stage));
    let Ok(forwarded) = dispatch(AuthBackend::AuthEndUsers, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production end-user stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthEndUsers, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));
}

#[tokio::test]
async fn prefetch_looks_up_then_checks() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let lookup = ScriptedLookup::new(Answer::Resolved);
    let stage = EndUserStageInput {
        context: master_context(&master_key),
        rule: rule(Arc::clone(&lookup), Some("alice")),
        route: MODEL_ROUTE.to_owned(),
        database: true,
    };
    prefetch(&stage).await.expect("the lookup and check pass");
    assert_eq!(lookup.lookups(), vec!["alice".to_owned()]);
    assert_eq!(lookup.checks(), vec!["alice /v1/files".to_owned()]);

    // an absent id, an absent lookup or an absent database never reaches the collaborator
    let skipped = ScriptedLookup::new(Answer::Failure);
    let no_id = EndUserStageInput {
        context: master_context(&master_key),
        rule: rule(Arc::clone(&skipped), None),
        route: MODEL_ROUTE.to_owned(),
        database: true,
    };
    prefetch(&no_id).await.expect("an absent id is skipped");
    let no_database = EndUserStageInput {
        context: master_context(&master_key),
        rule: rule(Arc::clone(&skipped), Some("alice")),
        route: MODEL_ROUTE.to_owned(),
        database: false,
    };
    prefetch(&no_database)
        .await
        .expect("an absent database is skipped");
    assert!(skipped.lookups().is_empty());

    let missing = ScriptedLookup::new(Answer::Missing);
    let absent_row = EndUserStageInput {
        context: master_context(&master_key),
        rule: rule(Arc::clone(&missing), Some("alice")),
        route: MODEL_ROUTE.to_owned(),
        database: true,
    };
    prefetch(&absent_row)
        .await
        .expect("an absent row is skipped");
    assert!(missing.checks().is_empty());
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

fn application(config: &Config, rule: Arc<EndUserRule>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule))
}

async fn run_request(config: &Config, rule: Arc<EndUserRule>) -> (StatusCode, String, bool) {
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
    let lookup = ScriptedLookup::new(Answer::Budget);
    let (status, body, reached) = run_request(
        &config_with_database(),
        rule(Arc::clone(&lookup), Some("alice")),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("\"type\":\"budget_exceeded\""), "{body}");
    assert!(body.contains("\"param\":null"), "{body}");
    assert!(body.contains("\"code\":\"422\""), "{body}");
    assert!(
        body.contains("ExceededBudget: End User=alice over budget. Spend=150, Budget=100"),
        "{body}"
    );
    assert!(!reached);
    assert_eq!(lookup.lookups(), vec!["alice".to_owned()]);
    assert_eq!(lookup.checks(), vec!["alice /v1/files".to_owned()]);

    let failing = ScriptedLookup::new(Answer::Failure);
    let (failure_status, failure_body, failure_reached) = run_request(
        &config_with_database(),
        rule(Arc::clone(&failing), Some("alice")),
    )
    .await;
    assert_eq!(failure_status, StatusCode::UNAUTHORIZED);
    assert!(
        failure_body.contains("\"type\":\"auth_error\""),
        "{failure_body}"
    );
    assert!(
        failure_body.contains("\"param\":\"None\""),
        "{failure_body}"
    );
    assert!(failure_body.contains("\"code\":\"401\""), "{failure_body}");
    assert!(
        failure_body.contains("Authentication Error, the end-user lookup failed"),
        "{failure_body}"
    );
    assert!(!failure_reached);
    assert!(failing.checks().is_empty());
}

#[tokio::test]
async fn resolved_end_user_reaches_the_handler() {
    let lookup = ScriptedLookup::new(Answer::Resolved);
    let (status, _body, reached) = run_request(
        &config_with_database(),
        rule(Arc::clone(&lookup), Some("alice")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
    assert_eq!(lookup.lookups(), vec!["alice".to_owned()]);
    assert_eq!(lookup.checks(), vec!["alice /v1/files".to_owned()]);

    // without a database the same rule resolves nothing and the request still reaches the handler
    let skipped = ScriptedLookup::new(Answer::Failure);
    let (skip_status, _skip_body, skip_reached) = run_request(
        &config_with_master_key(),
        rule(Arc::clone(&skipped), Some("alice")),
    )
    .await;
    assert_eq!(skip_status, StatusCode::OK);
    assert!(skip_reached);
    assert!(skipped.lookups().is_empty());
}
