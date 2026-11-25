#![cfg(test)]

use std::collections::BTreeMap;
use std::mem::take;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::middleware;
use axum::routing::post;
use betterlite_auth::auth_key_budgets::alerts::{BudgetAlert, BudgetAlertKind, BudgetAlertSink};
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::current_spend::{BUDGET_UNVERIFIABLE_DETAIL, SpendQuery};
use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::auth_key_budgets::Settings;
use betterlite_config::sections::auth_route_patterns::Settings as RoutePatternSettings;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use chrono::{DateTime, TimeZone as _, Utc};
use rust_decimal::Decimal;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{KeyBudgetRule, KeyBudgetStageInput, prefetch};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::auth::auth_route_patterns::{AuthSettings, stage as auth_route_patterns_stage};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";
const MASTER_KEY: &str = "secret-master-key";
const ALERT_FRACTION_VARIABLE: &str = "EMAIL_BUDGET_ALERT_MAX_SPEND_ALERT_PERCENTAGE";

fn config(environment: &[(&str, &str)], source: &str) -> Config {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(source)
        .expect("the test config loads")
}

fn config_with_master_key() -> Config {
    config(&[], "general_settings:\n  master_key: secret-master-key\n")
}

fn key(fields: &str) -> UserAPIKeyAuth {
    let JsonData::Object(map) = serde_json::from_str(fields).expect("the key fixture is JSON")
    else {
        panic!("the key fields must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).expect("the key fixture constructs")
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap()
}

fn rule(fields: &str) -> KeyBudgetRule {
    KeyBudgetRule {
        auth: Some(key(fields)),
        user_email: None,
        reader: None,
        alerts: None,
        durations: None,
        throttle_pct: None,
        clock: now,
        applied: Mutex::new(None),
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

struct ScriptedReader {
    spends: Mutex<Vec<Decimal>>,
    starts: Mutex<Vec<Option<Timestamp>>>,
}

impl ScriptedReader {
    fn new(spends: Vec<Decimal>) -> Arc<Self> {
        Arc::new(Self {
            spends: Mutex::new(spends),
            starts: Mutex::new(Vec::new()),
        })
    }

    fn starts(&self) -> Vec<Option<Timestamp>> {
        self.starts.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendReader for ScriptedReader {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        self.starts.lock().unwrap().push(query.window.start);
        let mut spends = self.spends.lock().unwrap();
        assert!(!spends.is_empty(), "the reader ran out of scripted spends");
        Ok(spends.remove(0))
    }
}

struct UnverifiableReader;

#[async_trait]
impl SpendReader for UnverifiableReader {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        Err(SpendReadError::Unverifiable {
            counter_key: query.counter_key.to_owned(),
        })
    }
}

#[derive(Default)]
struct RecordingSink {
    alerts: Mutex<Vec<BudgetAlert>>,
}

impl RecordingSink {
    fn take(&self) -> Vec<BudgetAlert> {
        take(&mut *self.alerts.lock().unwrap())
    }
}

impl BudgetAlertSink for RecordingSink {
    fn schedule(&self, alert: BudgetAlert) {
        self.alerts.lock().unwrap().push(alert);
    }
}

struct FixedDurations;

impl DurationParser for FixedDurations {
    fn seconds(&self, spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        match spec {
            "1m" => Some(60_i64),
            "1h" => Some(3_600_i64),
            _ => None,
        }
    }
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthKeyBudgets));
    // the config section reaches the state slot the stage reads
    let with_settings = config(
        &[(ALERT_FRACTION_VARIABLE, "0.5")],
        "general_settings:\n  master_key: secret-master-key\nlitellm_settings:\n  default_key_max_budget_alert_emails:\n    \"80\":\n      - owner@example.com\n",
    );
    let state = state_for(&with_settings);
    let settings = Arc::clone(&state.auth_key_budgets.settings);
    assert_eq!(settings.alert_fraction.to_bits(), 0.5_f64.to_bits());
    assert_eq!(
        settings
            .default_alert_emails
            .as_ref()
            .expect("the global map loads")
            .len(),
        1
    );

    // the rule's debug output names the type and never the key it resolved
    let debugged = rule(r#"{"token":"sk-debug","spend":1,"max_budget":10}"#);
    assert!(format!("{debugged:?}").contains("KeyBudgetRule"));
    assert!(!format!("{debugged:?}").contains("sk-debug"));

    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let budget_stage = KeyBudgetStageInput {
        context: master_context(&master_key),
        rule: Arc::new(rule(
            r#"{"token":"sk-registered","spend":1,"max_budget":10}"#,
        )),
        settings,
    };
    let input = StageInput::AuthKeyBudgets(Box::new(budget_stage));
    let Ok(forwarded) = dispatch(AuthBackend::AuthKeyBudgets, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production key budget stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthKeyBudgets, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));
}

#[tokio::test]
async fn master_key_request_without_server_state_reaches_the_handler() {
    let auth = AuthSettings {
        master_key: Some(SecretString::from(MASTER_KEY.to_owned())),
        routes: RoutePatternSettings {
            public_routes: Vec::new(),
        },
    };
    let reached = Arc::new(AtomicBool::new(false));
    let application =
        Router::new()
            .route(MODEL_ROUTE, post(mark_called))
            .layer(middleware::from_fn_with_state(
                auth,
                auth_route_patterns_stage,
            ));
    let response = application
        .oneshot(request_to(&reached))
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(reached.load(Ordering::Acquire));
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

fn application(config: &Config, rule: Arc<KeyBudgetRule>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule))
}

async fn run_request(config: &Config, rule: Arc<KeyBudgetRule>) -> (StatusCode, String, bool) {
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

fn reader_rule(fields: &str, spends: Vec<Decimal>) -> Arc<KeyBudgetRule> {
    Arc::new(KeyBudgetRule {
        reader: Some(ScriptedReader::new(spends)),
        ..rule(fields)
    })
}

#[tokio::test]
async fn failure_stops_downstream() {
    let (status, body, reached) = run_request(
        &config_with_master_key(),
        reader_rule(
            r#"{"token":"sk-denied","spend":0,"max_budget":5}"#,
            vec![Decimal::from(6_i64)],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("\"type\":\"budget_exceeded\""), "{body}");
    assert!(
        body.contains("Budget has been exceeded! Key=key Current cost: 6, Max budget: 5"),
        "{body}"
    );
    assert!(body.contains("\"param\":null"), "{body}");
    assert!(body.contains("\"code\":\"422\""), "{body}");
    assert!(!reached);
}

#[tokio::test]
async fn invalid_expiry_is_unauthorized() {
    let (status, body, reached) = run_request(
        &config_with_master_key(),
        Arc::new(rule(
            r#"{"token":"sk-bad-expiry","max_budget":100,"metadata":{"temp_budget_increase":10,"temp_budget_expiry":"garbage"}}"#,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(
        body.contains(
            "Authentication Error, the temporary budget expiry is not an ISO 8601 timestamp"
        ),
        "{body}"
    );
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(!reached);
}

#[tokio::test]
async fn malformed_alert_emails_is_unauthorized() {
    let (status, body, reached) = run_request(
        &config_with_master_key(),
        Arc::new(rule(
            r#"{"token":"sk-bad-alerts","spend":1,"max_budget":10,"metadata":{"max_budget_alert_emails":"owner@example.com"}}"#,
        )),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(
        body.contains(
            "Authentication Error, max_budget_alert_emails must be a mapping of thresholds to recipients"
        ),
        "{body}"
    );
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(!reached);
}

#[tokio::test]
async fn unverifiable_spend_is_unavailable() {
    let unverifiable = Arc::new(KeyBudgetRule {
        reader: Some(Arc::new(UnverifiableReader)),
        ..rule(r#"{"token":"sk-unverifiable","spend":0,"max_budget":5}"#)
    });
    let (status, body, reached) = run_request(&config_with_master_key(), unverifiable).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains(BUDGET_UNVERIFIABLE_DETAIL), "{body}");
    assert!(!reached);
}

#[tokio::test]
async fn budgets_without_reader_are_unverifiable() {
    let (status, body, reached) = run_request(
        &config_with_master_key(),
        Arc::new(rule(r#"{"token":"sk-no-reader","spend":0,"max_budget":5}"#)),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains(BUDGET_UNVERIFIABLE_DETAIL), "{body}");
    assert!(!reached);
}

#[tokio::test]
async fn window_denial_uses_rule_durations() {
    let windowed = Arc::new(KeyBudgetRule {
        reader: Some(ScriptedReader::new(vec![Decimal::from(7_i64)])),
        durations: Some(Arc::new(FixedDurations)),
        ..rule(
            r#"{"token":"sk-window","spend":0,"budget_limits":[{"budget_duration":"1m","max_budget":5}]}"#,
        )
    });
    let (status, body, reached) = run_request(&config_with_master_key(), windowed).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body.contains("ExceededBudget: Key over 1m budget. Spend=$7.0000, Limit=$5.00"),
        "{body}"
    );
    assert!(!reached);
}

#[tokio::test]
async fn temporary_increase_applies() {
    let increased = Arc::new(KeyBudgetRule {
        reader: Some(ScriptedReader::new(vec![Decimal::from(130_i64)])),
        ..rule(
            r#"{"token":"sk-temp","spend":0,"max_budget":100,"metadata":{"temp_budget_increase":30,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
        )
    });
    let (status, body, reached) = run_request(&config_with_master_key(), increased).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body.contains("Current cost: 130, Max budget: 130"),
        "{body}"
    );
    assert!(!reached);
}

#[tokio::test]
async fn soft_alert_passes_without_denial() {
    let recorder = Arc::new(RecordingSink::default());
    let shared = Arc::clone(&recorder);
    let dispatched: Arc<dyn BudgetAlertSink> = shared;
    let soft = Arc::new(KeyBudgetRule {
        alerts: Some(dispatched),
        ..rule(r#"{"token":"sk-soft","spend":5,"soft_budget":5}"#)
    });
    let (status, _body, reached) = run_request(&config_with_master_key(), soft).await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
    let alerts = recorder.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, BudgetAlertKind::SoftBudget);
}

#[tokio::test]
async fn missing_duration_parser_leaves_the_start_unset() {
    let reader = ScriptedReader::new(vec![Decimal::from(1_i64)]);
    let shared = Arc::clone(&reader);
    let spend_reader: Arc<dyn SpendReader> = shared;
    let windowed = Arc::new(KeyBudgetRule {
        reader: Some(spend_reader),
        ..rule(
            r#"{"token":"sk-no-duration","spend":0,"budget_limits":[{"budget_duration":"1m","max_budget":5}]}"#,
        )
    });
    let (status, _body, reached) = run_request(&config_with_master_key(), windowed).await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
    assert_eq!(reader.starts(), vec![None]);
}

#[tokio::test]
async fn configured_alert_fraction_reaches_the_stage() {
    let configured = config(
        &[(ALERT_FRACTION_VARIABLE, "0.5")],
        "general_settings:\n  master_key: secret-master-key\n",
    );
    let recorder = Arc::new(RecordingSink::default());
    let shared = Arc::clone(&recorder);
    let dispatched: Arc<dyn BudgetAlertSink> = shared;
    let alerting = Arc::new(KeyBudgetRule {
        alerts: Some(dispatched),
        reader: Some(ScriptedReader::new(vec![Decimal::from(6_i64)])),
        ..rule(r#"{"token":"sk-fraction","spend":6,"max_budget":10}"#)
    });
    let (status, _body, reached) = run_request(&configured, alerting).await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
    let alerts = recorder.take();
    assert_eq!(alerts.len(), 2);
    assert_eq!(alerts[0].kind, BudgetAlertKind::MaxBudgetAlert);
    assert_eq!(alerts[1].kind, BudgetAlertKind::TokenBudget);
}

#[tokio::test]
async fn prefetch_records_the_checked_key() {
    let master_key = SecretString::from(MASTER_KEY.to_owned());
    let budget_rule = Arc::new(KeyBudgetRule {
        throttle_pct: Some(0.5_f64),
        reader: Some(ScriptedReader::new(vec![Decimal::from(6_i64)])),
        ..rule(
            r#"{"token":"sk-throttle","spend":0,"max_budget":5,"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":10}"#,
        )
    });
    let stage = KeyBudgetStageInput {
        context: master_context(&master_key),
        rule: Arc::clone(&budget_rule),
        settings: Arc::new(Settings::default()),
    };
    prefetch(&stage)
        .await
        .expect("the throttle permits the request");
    let applied = budget_rule
        .applied()
        .expect("the stage records the checked key");
    assert_eq!(applied.request_quota_view().throttle_pct, Some(0.5_f64));
}
