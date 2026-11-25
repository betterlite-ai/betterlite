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
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_model_patterns::view::{Deployment, DeploymentView};
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::current_spend::{BUDGET_UNVERIFIABLE_DETAIL, SpendQuery};
use betterlite_auth::membership_load::error::{MembershipLoadError, MembershipReadFailure};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::team_budget_admission::alerts::{TeamAlert, TeamAlertKind, TeamAlertSink};
use betterlite_auth::team_budget_admission::sources::{Membership, MembershipSource};
use betterlite_config::loader::ConfigLoader;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::database_availability::message::TRANSIENT_MESSAGE;
use betterlite_db::database_failures::deadline::{DbLookupDeadlineExceeded, DbLookupError};
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::postgres_store::errors::DbError as NativeDbError;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use chrono::{DateTime, TimeZone as _, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{TeamBudgetRule, TeamBudgetStageInput};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";
const MASTER_KEY: &str = "secret-master-key";
const MEMBER_KEY: &str = r#"{"token":"sk-member","user_id":"user-1","team_id":"team-1","team_alias":"Alpha","org_id":"org-1"}"#;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
}

fn key() -> UserAPIKeyAuth {
    let JsonData::Object(map) = serde_json::from_str(MEMBER_KEY).unwrap() else {
        panic!("the key fixture is an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

#[derive(Default)]
struct Recorded {
    spends: Mutex<Vec<Decimal>>,
    starts: Mutex<Vec<Option<Timestamp>>>,
    alerts: Mutex<Vec<TeamAlert>>,
}

#[async_trait]
impl SpendReader for Recorded {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        self.starts.lock().unwrap().push(query.window.start);
        Ok(self.spends.lock().unwrap().remove(0))
    }
}

impl TeamAlertSink for Recorded {
    fn schedule(&self, alert: TeamAlert) {
        self.alerts.lock().unwrap().push(alert);
    }
}

// the one membership failure the scripted load raises; later loads find no membership
struct FailingMembership(Mutex<Option<MembershipLoadError>>);

#[async_trait]
impl MembershipSource for FailingMembership {
    async fn team_membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        self.0.lock().unwrap().take().map_or(Ok(None), Err)
    }
}

fn rule(team: &str, membership: &str, model: &str, recorded: &Arc<Recorded>) -> TeamBudgetRule {
    let reader: Arc<dyn SpendReader> = Arc::<Recorded>::clone(recorded);
    let team_alerts: Arc<dyn TeamAlertSink> = Arc::<Recorded>::clone(recorded);
    TeamBudgetRule {
        team: Some(serde_json::from_str(team).unwrap()),
        auth: Some(key()),
        user_email: None,
        model: RequestedModels::One(model.to_owned()),
        membership: Membership::Loaded(serde_json::from_str(membership).unwrap()),
        view: None,
        provider: None,
        global_model_aliases: BTreeMap::new(),
        reader: Some(reader),
        memberships: None,
        defaults: None,
        member_alerts: None,
        team_alerts: Some(team_alerts),
        durations: None,
        clock: now,
    }
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

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

// the real router with its auth chain, a handler that records whether it ran, and the rule the auth service would insert
async fn run_request(rule: TeamBudgetRule) -> (StatusCode, String, bool) {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .unwrap();
    let application = routes::router(
        state_for(&config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::new(rule)));
    let reached = Arc::new(AtomicBool::new(false));
    let mut request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(Arc::clone(&reached));
    let response = application.oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        String::from_utf8(body.to_vec()).unwrap(),
        reached.load(Ordering::Acquire),
    )
}

fn lookup_failure(lookup: DbLookupError) -> MembershipLoadError {
    MembershipLoadError::Read(MembershipReadFailure::Table {
        source: Box::new(AuthReadError::Lookup(lookup)),
    })
}

fn deadline_failure() -> MembershipLoadError {
    lookup_failure(DbLookupError::Deadline(DbLookupDeadlineExceeded {
        lookup: "team_membership".to_owned(),
        deadline_seconds: 5.0,
        cause: None,
    }))
}

fn database_fault(error_type: ErrorType) -> DbError {
    DbError {
        error_type,
        message: "the database refused the membership read".to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

async fn run_membership_failure(failure: MembershipLoadError) -> (StatusCode, String, bool) {
    let recorded = Arc::new(Recorded::default());
    let mut failing = rule(r#"{"team_id":"team-1"}"#, "null", "gpt-4o", &recorded);
    failing.membership = Membership::Unloaded;
    failing.memberships = Some(Arc::new(FailingMembership(Mutex::new(Some(failure)))));
    run_request(failing).await
}

fn assert_outage((status, body, reached): &(StatusCode, String, bool)) {
    assert_eq!(*status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body.contains("\"type\":\"no_db_connection\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"503\""), "{body}");
    assert!(body.contains(TRANSIENT_MESSAGE), "{body}");
    assert!(!*reached);
}

#[tokio::test]
async fn membership_database_outages_are_service_unavailable() {
    assert_outage(&run_membership_failure(deadline_failure()).await);
    assert_outage(
        &run_membership_failure(lookup_failure(DbLookupError::Database(database_fault(
            ErrorType::Connect,
        ))))
        .await,
    );
    assert_outage(
        &run_membership_failure(MembershipLoadError::Read(MembershipReadFailure::Access(
            Box::new(NativeDbError::NoDbConnected),
        )))
        .await,
    );
    assert_outage(
        &run_membership_failure(MembershipLoadError::Shared(Arc::new(deadline_failure()))).await,
    );
}

#[tokio::test]
async fn membership_ordinary_failures_stay_unauthorized() {
    let (data_status, data_body, data_reached) = run_membership_failure(lookup_failure(
        DbLookupError::Database(database_fault(ErrorType::Data)),
    ))
    .await;
    assert_eq!(data_status, StatusCode::UNAUTHORIZED, "{data_body}");
    assert!(data_body.contains("\"type\":\"auth_error\""), "{data_body}");
    assert!(!data_reached);
    let (lost_status, lost_body, lost_reached) =
        run_membership_failure(MembershipLoadError::LoadLost).await;
    assert_eq!(lost_status, StatusCode::UNAUTHORIZED, "{lost_body}");
    assert!(lost_body.contains("\"type\":\"auth_error\""), "{lost_body}");
    assert!(!lost_reached);
}

#[test]
fn stage_is_registered() {
    let position = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::TeamBudgetAdmission);
    let after_keys = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::AuthKeyBudgets)
        .map(|index| index.saturating_add(1));
    assert_eq!(position, after_keys);
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let input = StageInput::TeamBudgetAdmission(Box::new(TeamBudgetStageInput {
        context: master_context(&master_key),
        rule: Arc::new(TeamBudgetRule::default()),
        max_recurse_depth: 100,
    }));
    let Ok(forwarded) = dispatch(AuthBackend::TeamBudgetAdmission, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production team budget stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");
    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::TeamBudgetAdmission, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));
    // the rule's debug output names the team and never the key it resolved
    let recorded = Arc::new(Recorded::default());
    let debugged = rule(r#"{"team_id":"team-1"}"#, "null", "gpt-4o", &recorded);
    assert!(format!("{debugged:?}").contains("team-1"));
    assert!(!format!("{debugged:?}").contains("sk-member"));
}

#[tokio::test]
async fn failure_stops_downstream() {
    let recorded = Arc::new(Recorded::default());
    *recorded.spends.lock().unwrap() = vec![Decimal::new(1001, 2)];
    let denied = rule(
        r#"{"team_id":"team-1","max_budget":10}"#,
        "null",
        "gpt-4o",
        &recorded,
    );
    let (status, body, reached) = run_request(denied).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("\"type\":\"budget_exceeded\""), "{body}");
    assert!(body.contains("\"param\":null"), "{body}");
    assert!(body.contains("\"code\":\"422\""), "{body}");
    assert!(!reached);
    let alerts = recorded.alerts.lock().unwrap().clone();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, TeamAlertKind::TeamBudget);
}

#[tokio::test]
async fn team_equality_reaches_the_handler() {
    let recorded = Arc::new(Recorded::default());
    *recorded.spends.lock().unwrap() = vec![Decimal::TEN];
    let admitted = rule(
        r#"{"team_id":"team-1","max_budget":10}"#,
        "null",
        "gpt-4o",
        &recorded,
    );
    let (status, body, reached) = run_request(admitted).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(reached);
    assert!(recorded.alerts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn router_view_and_global_alias_admit_the_member_scope() {
    let recorded = Arc::new(Recorded::default());
    let grouped = r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["group-a"]}}"#;
    let mut viewed = rule(r#"{"team_id":"team-1"}"#, grouped, "gpt-4o", &recorded);
    viewed.view = Some(Arc::new(DeploymentView {
        deployments: vec![Deployment {
            model_name: "gpt-4o".to_owned(),
            access_groups: vec!["group-a".to_owned()],
            team_id: None,
            team_public_model_name: None,
        }],
        model_group_alias: IndexMap::new(),
    }));
    let (status, body, reached) = run_request(viewed).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(reached);
    // without the router view the access-group name is not a model, so B747 denies it
    let unviewed = rule(r#"{"team_id":"team-1"}"#, grouped, "gpt-4o", &recorded);
    let (denied, denied_body, _reached) = run_request(unviewed).await;
    assert_eq!(denied, StatusCode::FORBIDDEN, "{denied_body}");
    // the router's global alias target is what the allowlist names
    let aliased = r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["gpt-4o"]}}"#;
    let mut alias_rule = rule(r#"{"team_id":"team-1"}"#, aliased, "fast", &recorded);
    alias_rule.global_model_aliases = BTreeMap::from([("fast".to_owned(), "gpt-4o".to_owned())]);
    let (alias_status, alias_body, alias_reached) = run_request(alias_rule).await;
    assert_eq!(alias_status, StatusCode::OK, "{alias_body}");
    assert!(alias_reached);
}

#[tokio::test]
async fn member_equality_and_scope_stop_downstream() {
    let recorded = Arc::new(Recorded::default());
    *recorded.spends.lock().unwrap() = vec![Decimal::from(5_i64)];
    let member_cap =
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"max_budget":5}}"#;
    let at_cap = rule(r#"{"team_id":"team-1"}"#, member_cap, "gpt-4o", &recorded);
    let (status, body, reached) = run_request(at_cap).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(!reached);
    let scoped = r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["gpt-4o"]}}"#;
    let outside = rule(r#"{"team_id":"team-1"}"#, scoped, "claude-3", &recorded);
    let (scope_status, scope_body, scope_reached) = run_request(outside).await;
    assert_eq!(scope_status, StatusCode::FORBIDDEN, "{scope_body}");
    assert!(
        scope_body.contains("\"type\":\"team_model_access_denied\""),
        "{scope_body}"
    );
    assert!(scope_body.contains("\"param\":\"model\""), "{scope_body}");
    assert!(scope_body.contains("\"code\":\"403\""), "{scope_body}");
    assert!(!scope_body.contains("user-1"), "{scope_body}");
    assert!(!scope_reached);
}

#[tokio::test]
async fn budgets_without_reader_are_unavailable() {
    let recorded = Arc::new(Recorded::default());
    let unread = TeamBudgetRule {
        reader: None,
        ..rule(
            r#"{"team_id":"team-1","max_budget":10}"#,
            "null",
            "gpt-4o",
            &recorded,
        )
    };
    let (status, body, reached) = run_request(unread).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains(BUDGET_UNVERIFIABLE_DETAIL), "{body}");
    assert!(!reached);
}

#[tokio::test]
async fn unloaded_membership_without_database_is_unauthorized() {
    let recorded = Arc::new(Recorded::default());
    let unloaded = TeamBudgetRule {
        membership: Membership::Unloaded,
        ..rule(r#"{"team_id":"team-1"}"#, "null", "gpt-4o", &recorded)
    };
    let (status, body, reached) = run_request(unloaded).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(!reached);
}

#[tokio::test]
async fn missing_duration_parser_leaves_the_start_unset() {
    let recorded = Arc::new(Recorded::default());
    *recorded.spends.lock().unwrap() = vec![Decimal::ONE];
    let windowed = rule(
        r#"{"team_id":"team-1","budget_limits":[{"budget_duration":"1h","max_budget":5}]}"#,
        "null",
        "gpt-4o",
        &recorded,
    );
    let (status, body, reached) = run_request(windowed).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(reached);
    assert_eq!(*recorded.starts.lock().unwrap(), vec![None]);
}
