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
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::current_spend::{BUDGET_UNVERIFIABLE_DETAIL, SpendQuery};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::team_budget_admission::alerts::{TeamAlert, TeamAlertKind, TeamAlertSink};
use betterlite_auth::team_budget_admission::sources::Membership;
use betterlite_config::loader::ConfigLoader;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use chrono::{DateTime, TimeZone as _, Utc};
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

fn rule(team: &str, membership: &str, model: &str, recorded: &Arc<Recorded>) -> TeamBudgetRule {
    let reader: Arc<dyn SpendReader> = Arc::clone(recorded);
    let team_alerts: Arc<dyn TeamAlertSink> = Arc::clone(recorded);
    TeamBudgetRule {
        team: Some(serde_json::from_str(team).unwrap()),
        auth: Some(key()),
        user_email: None,
        model: RequestedModels::One(model.to_owned()),
        membership: Membership::Loaded(serde_json::from_str(membership).unwrap()),
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
        lookup: BTreeMap::new(),
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

#[test]
fn stage_is_registered() {
    let position = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::TeamBudgetAdmission);
    let after_keys = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::AuthKeyBudgets)
        .map(|index| index + 1);
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
    let (status, body, reached) = run_request(outside).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.contains("\"type\":\"team_model_access_denied\""),
        "{body}"
    );
    assert!(body.contains("\"param\":\"model\""), "{body}");
    assert!(body.contains("\"code\":\"403\""), "{body}");
    assert!(!body.contains("user-1"), "{body}");
    assert!(!reached);
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
