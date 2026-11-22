#![cfg(test)]

use std::collections::BTreeMap;
use std::io::Result as IoResult;
use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, PortError};
use betterlite_auth::jwt_claims::handler::{AgentLookup, JwtHandler};
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::auth_user_lookup::DefaultUserParams;
use betterlite_config::sections::jwt_settings;
use betterlite_config::sections::mock_testing_params;
use betterlite_config::sections::prompt_injection::PromptInjectionParams;
use betterlite_config::ui_settings::UiSettingsDict;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_guardrails::prompt_injection::error::PromptInjectionError;
use betterlite_guardrails::prompt_injection::hook::PromptInjectionDetection;
use betterlite_guardrails::prompt_injection::pool::HeuristicsPool;
use betterlite_models::decisions::Json;
use betterlite_models::key_digest::hashing::hash_token;
use betterlite_server::coord_redis_boot::settings::CoordinationGeneralSettings;
use betterlite_server::proxy_request_contracts::coordination_redis::BoolOrText;
use betterlite_server::ui_settings_state::projection::GeneralSettingsTarget;
use betterlite_server::ui_settings_state::repository::UiSettingsRepository;
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::startup_checks::detectors::attach_router_to_detectors;
use crate::startup_checks::error::StartupCheckError;
use crate::startup_checks::jwt::init_jwt_auth;
use crate::startup_checks::presence::{DatabasePresence, RedisPresence};
use crate::startup_checks::proxy_budget::{
    LITELLM_PROXY_BUDGET_NAME, ProxyBudget, ProxyBudgetStore, seed_proxy_budget,
};
use crate::startup_checks::spend::{GlobalSpendFetch, warm_global_spend};
use crate::startup_checks::team_budgets::{TeamMemberBudgetUpdater, update_default_team_budgets};
use crate::startup_checks::transaction_buffer::{check_transaction_buffer, wants_buffer_redis};
use crate::startup_checks::tuning::{AUTO_ROUTER_COMPLEXITY_PREFIX, Deployment};
use crate::startup_checks::ui_settings::sync_ui_settings;
use crate::startup_checks::warnings::{
    warn_budget_without_db, warn_fail_closed_without_redis, warn_mock_testing,
};

mod tuning;

const EMPTY_FINGERPRINT: &str = "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).unwrap()
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_max_level(Level::TRACE)
        .without_time()
        .with_ansi(false)
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

fn json(text: &str) -> Json {
    serde_json::from_str::<Json>(text).unwrap()
}

fn deployment(name: &str, config: Option<Json>) -> Deployment {
    Deployment {
        model_name: Some(name.to_owned()),
        model: Some(format!("{AUTO_ROUTER_COMPLEXITY_PREFIX}/{name}")),
        tags: Vec::new(),
        model_info_id: None,
        db_model: false,
        complexity_router_config: config,
    }
}

const fn buffer_settings(flag: BoolOrText) -> CoordinationGeneralSettings {
    CoordinationGeneralSettings {
        coordination_redis: None,
        use_redis_transaction_buffer: Some(flag),
    }
}

fn team_entry(team_id: &str, budget: Option<JsonData>) -> JsonData {
    let mut fields = BTreeMap::new();
    fields.insert("team_id".to_owned(), JsonData::String(team_id.to_owned()));
    if let Some(budget) = budget {
        fields.insert("max_budget_in_team".to_owned(), budget);
    }
    JsonData::Object(fields)
}

fn params_with_teams(teams: JsonData) -> DefaultUserParams {
    DefaultUserParams {
        values: IndexMap::from([("teams".to_owned(), teams)]),
    }
}

// warnings

#[test]
fn budget_warning_fires_only_without_database_and_above_zero() {
    let (logs, _guard) = capture_logs();
    warn_budget_without_db(Some(10.0), DatabasePresence::Absent);
    warn_budget_without_db(None, DatabasePresence::Absent);
    warn_budget_without_db(Some(0.0), DatabasePresence::Absent);
    warn_budget_without_db(Some(-1.0), DatabasePresence::Absent);
    warn_budget_without_db(Some(10.0), DatabasePresence::Present);
    let contents = logs.contents();
    assert_eq!(contents.matches("A proxy-wide budget").count(), 1);
    assert!(contents.contains("litellm.max_budget=10.0"));
}

#[test]
fn budget_warning_keeps_litellm_text() {
    let (logs, _guard) = capture_logs();
    warn_budget_without_db(Some(3.0), DatabasePresence::Absent);
    let expected = "A proxy-wide budget (litellm.max_budget=3.0) is configured but no database is connected, so the budget will NOT be enforced and requests will never be blocked. Set DATABASE_URL or general_settings.database_url and restart. Redis and fail_closed_budget_enforcement do not cover the proxy-wide budget because there is no global spend counter; Redis alone is not a substitute.";
    assert!(logs.contents().contains(expected));
}

#[test]
fn fail_closed_warning_fires_only_when_enabled_without_redis() {
    let (logs, _guard) = capture_logs();
    warn_fail_closed_without_redis(true, RedisPresence::Present);
    warn_fail_closed_without_redis(false, RedisPresence::Absent);
    assert!(
        !logs
            .contents()
            .contains("is enabled but no Redis is configured")
    );
    warn_fail_closed_without_redis(true, RedisPresence::Absent);
    let contents = logs.contents();
    assert_eq!(
        contents
            .matches("general_settings.fail_closed_rate_limit_enforcement is enabled")
            .count(),
        1
    );
    assert!(contents.contains(
        "Configure general_settings.coordination_redis (or REDIS_HOST/REDIS_PORT/REDIS_PASSWORD) to share the counters across pods and make the setting effective."
    ));
}

#[test]
fn mock_testing_banner_lists_each_gated_name() {
    let (logs, _guard) = capture_logs();
    warn_mock_testing(
        &mock_testing_params::Settings { allowed: true },
        &["mock_timeout", "mock_delay"],
    );
    let banner = "=".repeat(72);
    let expected = format!(
        "\n{banner}\n DANGEROUS SETTING ENABLED\n general_settings.dangerously_allow_mock_testing_request_params = true\n\n Any caller with a valid key on this proxy can now inject synthetic\n failures and latency into their own requests using these body params:\n   mock_timeout\n   mock_delay\n\n A request using them consumes a connection and a concurrency slot\n without reaching a provider, and returns an error the caller chose.\n\n Intended for testing fallback chains. Do not leave enabled.\n{banner}"
    );
    assert!(logs.contents().contains(&expected));
}

#[test]
fn mock_testing_banner_stays_off_unless_exactly_enabled() {
    let (logs, _guard) = capture_logs();
    warn_mock_testing(
        &mock_testing_params::Settings { allowed: false },
        &["mock_timeout"],
    );
    assert!(!logs.contents().contains("DANGEROUS SETTING ENABLED"));
}

// transaction buffer

#[test]
fn true_bool_without_redis_stops_startup_with_litellm_message() {
    let settings = buffer_settings(BoolOrText::Bool(true));
    let error = check_transaction_buffer(&settings, RedisPresence::Absent).unwrap_err();
    let expected = "`use_redis_transaction_buffer` is enabled in general_settings but no Redis is configured. This will cause spend updates to not be tracked. Add a Redis cache in litellm_settings:\n\nlitellm_settings:\n  cache: true\n  cache_params:\n    type: redis\n    url: os.environ/REDIS_URL\n\nor set REDIS_* environment variables (e.g. REDIS_HOST, REDIS_PORT, REDIS_PASSWORD, or REDIS_URL) to use a standalone Redis for the transaction buffer.";
    assert_eq!(error.to_string(), expected);
    assert!(matches!(error, StartupCheckError::TransactionBuffer));
}

#[rstest]
#[case::lower_true("true")]
#[case::upper_true("TRUE")]
#[case::padded_true(" True ")]
fn string_flag_truthy_requires_redis(#[case] flag: &str) {
    let settings = buffer_settings(BoolOrText::Text(flag.to_owned()));
    assert!(wants_buffer_redis(&settings));
    check_transaction_buffer(&settings, RedisPresence::Absent).unwrap_err();
    check_transaction_buffer(&settings, RedisPresence::Present).unwrap();
}

#[rstest]
#[case::lower_false("false")]
#[case::no("no")]
#[case::empty("")]
fn string_flag_falsy_leaves_startup_running(#[case] flag: &str) {
    let settings = buffer_settings(BoolOrText::Text(flag.to_owned()));
    assert!(!wants_buffer_redis(&settings));
    check_transaction_buffer(&settings, RedisPresence::Absent).unwrap();
}

#[test]
fn unset_transaction_buffer_flag_stays_off() {
    let unset = CoordinationGeneralSettings::default();
    assert!(!wants_buffer_redis(&unset));
    check_transaction_buffer(&unset, RedisPresence::Absent).unwrap();
}

// jwt

struct StubLookup;

impl AgentLookup for StubLookup {}

#[test]
fn jwt_settings_resolve_only_top_level_environment_values() {
    let source = r#"
general_settings:
  litellm_jwtauth:
    user_id_jwt_field: os.environ/USER_ID_FIELD
    user_email_jwt_field: plain@example.com
    user_id_upsert: true
    org_id_jwt_field: org
    issuers:
      - issuer: https://idp.example.com
        jwks_url: os.environ/JWKS_URL
        disable_audience_validation: true
"#;
    let lookup = BTreeMap::from([("USER_ID_FIELD".to_owned(), "sub".to_owned())]);
    let loaded = ConfigLoader { lookup }.load(source).unwrap();
    let settings = loaded.sections.jwt_settings;
    assert_eq!(settings.claims.user_id_jwt_field.as_deref(), Some("sub"));
    assert_eq!(settings.claims.org_id_jwt_field.as_deref(), Some("org"));
    let issuer = &settings.routing.issuers.as_ref().unwrap()[0];
    assert_eq!(issuer.jwks_url.as_deref(), Some("os.environ/JWKS_URL"));

    let handler = init_jwt_auth(&settings, Arc::new(StubLookup), JwtHandler::default());
    assert_eq!(handler.settings.user_id_jwt_field.as_deref(), Some("sub"));
    assert_eq!(
        handler.settings.user_email_jwt_field.as_deref(),
        Some("plain@example.com")
    );
    assert!(handler.settings.user_id_upsert);
    assert_eq!(handler.settings.org_id_jwt_field.as_deref(), Some("org"));
    assert_eq!(
        handler.trusted_issuers,
        vec!["https://idp.example.com".to_owned()]
    );
    assert_eq!(handler.leeway, 0);
    assert!(handler.agent_lookup.is_some());
}

#[test]
fn jwt_defaults_still_bind_the_agent_lookup() {
    let settings = jwt_settings::Settings::default();
    let handler = init_jwt_auth(&settings, Arc::new(StubLookup), JwtHandler::default());
    assert_eq!(handler.settings, JwtAuthSettings::default());
    assert!(handler.trusted_issuers.is_empty());
    assert!(handler.agent_lookup.is_some());
}

// prompt injection detectors

fn detector(api_name: &str) -> PromptInjectionDetection {
    let params = PromptInjectionParams {
        heuristics_check: false,
        vector_db_check: false,
        llm_api_check: true,
        llm_api_name: Some(api_name.to_owned()),
        llm_api_system_prompt: None,
        llm_api_fail_call_string: None,
        reject_as_response: None,
    };
    PromptInjectionDetection::new(
        Some(params),
        Decimal::new(7, 1),
        HeuristicsPool::new(1, Duration::from_secs(1)),
    )
}

#[test]
fn every_prompt_injection_detector_receives_the_model_list() {
    let names = vec!["listed".to_owned()];
    let mut detectors = vec![detector("listed"), detector("missing")];
    let error = attach_router_to_detectors(&mut detectors, Some(&names)).unwrap_err();
    assert!(matches!(
        error,
        StartupCheckError::PromptInjection(PromptInjectionError::InvalidLlmApiName)
    ));
    let mut passing = vec![detector("listed")];
    attach_router_to_detectors(&mut passing, Some(&names)).unwrap();
}

// proxy budget

#[derive(Default)]
struct BudgetStore {
    upserts: Mutex<Vec<(String, Decimal, String)>>,
    backfills: Mutex<Vec<(String, Timestamp)>>,
    upsert_error: Mutex<Option<String>>,
    backfill_error: Mutex<Option<String>>,
}

#[async_trait]
impl ProxyBudgetStore for BudgetStore {
    async fn upsert_proxy_budget(
        &self,
        user_id: &str,
        max_budget: Decimal,
        budget_duration: &str,
    ) -> Result<(), PortError> {
        self.upserts.lock().unwrap().push((
            user_id.to_owned(),
            max_budget,
            budget_duration.to_owned(),
        ));
        match self.upsert_error.lock().unwrap().as_deref() {
            Some(message) => Err(PortError::new(message)),
            None => Ok(()),
        }
    }

    async fn backfill_null_budget_reset(
        &self,
        user_id: &str,
        reset_at: Timestamp,
    ) -> Result<(), PortError> {
        self.backfills
            .lock()
            .unwrap()
            .push((user_id.to_owned(), reset_at));
        match self.backfill_error.lock().unwrap().as_deref() {
            Some(message) => Err(PortError::new(message)),
            None => Ok(()),
        }
    }
}

struct FixedReset {
    outcome: Result<Timestamp, ()>,
}

impl BudgetResetCalculator for FixedReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        match &self.outcome {
            Ok(value) => Ok(*value),
            Err(()) => Err(PortError::new("bad duration")),
        }
    }
}

fn fixed_reset() -> Timestamp {
    Timestamp::Aware(DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z").unwrap())
}

const fn budget(
    max_budget: Decimal,
    duration: Option<&str>,
    database: DatabasePresence,
) -> ProxyBudget<'_> {
    ProxyBudget {
        max_budget,
        budget_duration: duration,
        database,
    }
}

#[tokio::test]
async fn seed_proxy_budget_refuses_missing_duration() {
    let store = Arc::new(BudgetStore::default());
    let reset = Arc::new(FixedReset {
        outcome: Ok(fixed_reset()),
    });
    let tasks = TaskTracker::new();
    let error = seed_proxy_budget(
        budget(Decimal::from(5), None, DatabasePresence::Present),
        Arc::clone(&store),
        reset,
        &tasks,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "budget_duration not set on Proxy. budget_duration is required to use max_budget."
    );
    tasks.close();
    tasks.wait().await;
    assert!(store.upserts.lock().unwrap().is_empty());
}

#[rstest]
#[case::no_database(Decimal::from(5), Some("30d"), DatabasePresence::Absent)]
#[case::zero_budget(Decimal::ZERO, Some("30d"), DatabasePresence::Present)]
#[case::negative_budget(Decimal::from(-1), Some("30d"), DatabasePresence::Present)]
#[tokio::test]
async fn seed_proxy_budget_skips_without_database_or_positive_budget(
    #[case] max_budget: Decimal,
    #[case] duration: Option<&'static str>,
    #[case] database: DatabasePresence,
) {
    let store = Arc::new(BudgetStore::default());
    let tasks = TaskTracker::new();
    seed_proxy_budget(
        budget(max_budget, duration, database),
        Arc::clone(&store),
        Arc::new(FixedReset {
            outcome: Ok(fixed_reset()),
        }),
        &tasks,
    )
    .unwrap();
    tasks.close();
    tasks.wait().await;
    assert!(store.upserts.lock().unwrap().is_empty());
    assert!(store.backfills.lock().unwrap().is_empty());
}

#[tokio::test]
async fn seed_proxy_budget_upserts_then_backfills_the_reset() {
    let store = Arc::new(BudgetStore::default());
    let tasks = TaskTracker::new();
    seed_proxy_budget(
        budget(Decimal::from(5), Some("30d"), DatabasePresence::Present),
        Arc::clone(&store),
        Arc::new(FixedReset {
            outcome: Ok(fixed_reset()),
        }),
        &tasks,
    )
    .unwrap();
    tasks.close();
    tasks.wait().await;
    assert_eq!(
        *store.upserts.lock().unwrap(),
        vec![(
            LITELLM_PROXY_BUDGET_NAME.to_owned(),
            Decimal::from(5),
            "30d".to_owned()
        )]
    );
    assert_eq!(
        *store.backfills.lock().unwrap(),
        vec![(LITELLM_PROXY_BUDGET_NAME.to_owned(), fixed_reset())]
    );
}

#[tokio::test]
async fn seed_proxy_budget_warns_and_skips_the_backfill_when_the_upsert_fails() {
    let store = Arc::new(BudgetStore::default());
    *store.upsert_error.lock().unwrap() = Some("no primary".to_owned());
    let (logs, _guard) = capture_logs();
    let tasks = TaskTracker::new();
    seed_proxy_budget(
        budget(Decimal::from(5), Some("30d"), DatabasePresence::Present),
        Arc::clone(&store),
        Arc::new(FixedReset {
            outcome: Ok(fixed_reset()),
        }),
        &tasks,
    )
    .unwrap();
    tasks.close();
    tasks.wait().await;
    assert_eq!(store.upserts.lock().unwrap().len(), 1);
    assert!(store.backfills.lock().unwrap().is_empty());
    assert!(
        logs.contents()
            .contains("Failed to upsert the proxy budget row: no primary")
    );
}

#[tokio::test]
async fn seed_proxy_budget_warns_when_the_backfill_fails() {
    let store = Arc::new(BudgetStore::default());
    *store.backfill_error.lock().unwrap() = Some("no replica".to_owned());
    let (logs, _guard) = capture_logs();
    let tasks = TaskTracker::new();
    seed_proxy_budget(
        budget(Decimal::from(5), Some("30d"), DatabasePresence::Present),
        Arc::clone(&store),
        Arc::new(FixedReset {
            outcome: Ok(fixed_reset()),
        }),
        &tasks,
    )
    .unwrap();
    tasks.close();
    tasks.wait().await;
    assert_eq!(store.backfills.lock().unwrap().len(), 1);
    assert!(
        logs.contents()
            .contains("Failed to backfill budget_reset_at on proxy admin row: no replica")
    );
}

#[tokio::test]
async fn seed_proxy_budget_warns_when_the_reset_time_cannot_be_computed() {
    let store = Arc::new(BudgetStore::default());
    let (logs, _guard) = capture_logs();
    let tasks = TaskTracker::new();
    seed_proxy_budget(
        budget(Decimal::from(5), Some("bad"), DatabasePresence::Present),
        Arc::clone(&store),
        Arc::new(FixedReset { outcome: Err(()) }),
        &tasks,
    )
    .unwrap();
    tasks.close();
    tasks.wait().await;
    assert!(store.backfills.lock().unwrap().is_empty());
    assert!(
        logs.contents()
            .contains("Failed to backfill budget_reset_at on proxy admin row: bad duration")
    );
}

// spend warm-up

#[derive(Default)]
struct RecordingFetch {
    calls: Mutex<usize>,
    error: Mutex<Option<String>>,
}

#[async_trait]
impl GlobalSpendFetch for RecordingFetch {
    async fn fetch_global_spend(&self) -> Result<(), PortError> {
        *self.calls.lock().unwrap() += 1;
        match self.error.lock().unwrap().as_deref() {
            Some(message) => Err(PortError::new(message)),
            None => Ok(()),
        }
    }
}

#[tokio::test]
async fn warm_global_spend_reads_once_under_the_budget_condition() {
    let fetch = Arc::new(RecordingFetch::default());
    let tasks = TaskTracker::new();
    warm_global_spend(
        Decimal::from(5),
        DatabasePresence::Present,
        Arc::clone(&fetch),
        &tasks,
    );
    warm_global_spend(
        Decimal::from(5),
        DatabasePresence::Absent,
        Arc::clone(&fetch),
        &tasks,
    );
    warm_global_spend(
        Decimal::ZERO,
        DatabasePresence::Present,
        Arc::clone(&fetch),
        &tasks,
    );
    tasks.close();
    tasks.wait().await;
    assert_eq!(*fetch.calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn warm_global_spend_swallows_errors_at_debug() {
    let fetch = Arc::new(RecordingFetch::default());
    *fetch.error.lock().unwrap() = Some("reader down".to_owned());
    let (logs, _guard) = capture_logs();
    let tasks = TaskTracker::new();
    warm_global_spend(Decimal::from(5), DatabasePresence::Present, fetch, &tasks);
    tasks.close();
    tasks.wait().await;
    assert!(
        logs.contents()
            .contains("Global spend cache warm-up at startup skipped or failed: reader down")
    );
}

// ui settings

struct SettingsRepo {
    row: Mutex<Option<String>>,
}

#[async_trait]
impl UiSettingsRepository for SettingsRepo {
    async fn stored_ui_settings(&self) -> Result<Option<String>, TableError> {
        match self.row.lock().unwrap().as_ref() {
            Some(text) => Ok(Some(text.clone())),
            None => Err(TableError::MissingRow),
        }
    }
}

fn settings_repo(row: Option<&str>) -> SettingsRepo {
    SettingsRepo {
        row: Mutex::new(row.map(str::to_owned)),
    }
}

#[tokio::test]
async fn ui_settings_sync_logs_the_applied_keys() {
    let repository = settings_repo(Some(
        r#"{"forward_client_headers_to_llm_api": true, "unrelated": 1}"#,
    ));
    let (logs, _guard) = capture_logs();
    let mut map = UiSettingsDict::new();
    let reader: &dyn UiSettingsRepository = &repository;
    sync_ui_settings(Some(reader), GeneralSettingsTarget::Map(&mut map)).await;
    assert_eq!(
        map.get("forward_client_headers_to_llm_api")
            .map(ToString::to_string),
        Some("true".to_owned())
    );
    assert!(logs.contents().contains(
        "Synced UI settings to general_settings on startup: ['forward_client_headers_to_llm_api']"
    ));
}

#[tokio::test]
async fn ui_settings_sync_stays_quiet_when_nothing_applies() {
    let repository = settings_repo(Some(r#"{"unrelated": 1}"#));
    let (logs, _guard) = capture_logs();
    let mut map = UiSettingsDict::new();
    let reader: &dyn UiSettingsRepository = &repository;
    sync_ui_settings(Some(reader), GeneralSettingsTarget::Map(&mut map)).await;
    assert!(!logs.contents().contains("Synced UI settings"));
}

#[tokio::test]
async fn ui_settings_sync_never_stops_startup_on_a_read_failure() {
    let repository = settings_repo(None);
    let (logs, _guard) = capture_logs();
    let mut map = UiSettingsDict::new();
    let reader: &dyn UiSettingsRepository = &repository;
    sync_ui_settings(Some(reader), GeneralSettingsTarget::Map(&mut map)).await;
    assert!(
        logs.contents()
            .contains("Could not refresh UI settings from the database")
    );
    assert!(!logs.contents().contains("Synced UI settings"));
}

#[tokio::test]
async fn ui_settings_sync_does_nothing_without_a_database() {
    let (logs, _guard) = capture_logs();
    let mut map = UiSettingsDict::new();
    sync_ui_settings(None, GeneralSettingsTarget::Map(&mut map)).await;
    assert!(map.is_empty());
    assert!(logs.contents().is_empty());
}

// default team budgets

#[derive(Default)]
struct RecordingUpdater {
    calls: Mutex<Vec<(String, Option<Decimal>, String)>>,
    fail_team: Option<String>,
}

#[async_trait]
impl TeamMemberBudgetUpdater for RecordingUpdater {
    async fn update_member_budget(
        &self,
        team_id: &str,
        max_budget_in_team: Option<Decimal>,
        admin_token: &SecretString,
    ) -> Result<(), PortError> {
        self.calls.lock().unwrap().push((
            team_id.to_owned(),
            max_budget_in_team,
            admin_token.expose_secret().to_owned(),
        ));
        if self.fail_team.as_deref() == Some(team_id) {
            return Err(PortError::new("team missing"));
        }
        Ok(())
    }
}

#[tokio::test]
async fn team_budgets_run_as_the_hashed_master_key() {
    let teams = JsonData::Array(vec![
        team_entry(
            "team-a",
            Some(JsonData::Number(serde_json::Number::from(12))),
        ),
        team_entry("team-b", None),
    ]);
    let params = params_with_teams(teams);
    let master_key = SecretString::from("sk-master".to_owned());
    let updater = RecordingUpdater::default();
    update_default_team_budgets(Some(&params), &master_key, &updater)
        .await
        .unwrap();
    let calls = updater.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "team-a");
    assert_eq!(calls[0].1, Some(Decimal::from(12)));
    assert_eq!(calls[1].0, "team-b");
    assert_eq!(calls[1].1, None);
    assert_eq!(calls[0].2, hash_token("sk-master"));
}

#[tokio::test]
async fn team_budgets_skip_without_a_team_list() {
    let updater = RecordingUpdater::default();
    let master_key = SecretString::from("sk-master".to_owned());
    update_default_team_budgets(None, &master_key, &updater)
        .await
        .unwrap();
    let absent = DefaultUserParams {
        values: IndexMap::new(),
    };
    update_default_team_budgets(Some(&absent), &master_key, &updater)
        .await
        .unwrap();
    let empty = params_with_teams(JsonData::Array(Vec::new()));
    update_default_team_budgets(Some(&empty), &master_key, &updater)
        .await
        .unwrap();
    assert!(updater.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn team_budgets_reject_a_team_list_of_the_wrong_shape() {
    let updater = RecordingUpdater::default();
    let master_key = SecretString::from("sk-master".to_owned());
    let not_a_list = params_with_teams(JsonData::String("team-a".to_owned()));
    let error = update_default_team_budgets(Some(&not_a_list), &master_key, &updater)
        .await
        .unwrap_err();
    assert!(matches!(error, StartupCheckError::TeamsNotAList));
    let missing_id = params_with_teams(JsonData::Array(vec![JsonData::Object(BTreeMap::new())]));
    let entry_error = update_default_team_budgets(Some(&missing_id), &master_key, &updater)
        .await
        .unwrap_err();
    assert!(matches!(entry_error, StartupCheckError::TeamEntry(_)));
    let string_budget = params_with_teams(JsonData::Array(vec![team_entry(
        "team-a",
        Some(JsonData::String("10".to_owned())),
    )]));
    let budget_error = update_default_team_budgets(Some(&string_budget), &master_key, &updater)
        .await
        .unwrap_err();
    assert!(matches!(budget_error, StartupCheckError::TeamEntry(_)));
    assert!(updater.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn team_budgets_log_a_failure_and_continue() {
    let teams = JsonData::Array(vec![
        team_entry(
            "team-a",
            Some(JsonData::Number(serde_json::Number::from(12))),
        ),
        team_entry("team-b", None),
    ]);
    let params = params_with_teams(teams);
    let master_key = SecretString::from("sk-master".to_owned());
    let updater = RecordingUpdater {
        calls: Mutex::new(Vec::new()),
        fail_team: Some("team-a".to_owned()),
    };
    let (logs, _guard) = capture_logs();
    update_default_team_budgets(Some(&params), &master_key, &updater)
        .await
        .unwrap();
    assert_eq!(updater.calls.lock().unwrap().len(), 2);
    assert!(logs.contents().contains(
        "Error updating team team-a with team member budget 12.0 with error: team missing, skipping.."
    ));
}

#[tokio::test]
async fn team_budgets_log_a_missing_budget_and_continue() {
    let no_budget = params_with_teams(JsonData::Array(vec![team_entry("team-c", None)]));
    let master_key = SecretString::from("sk-master".to_owned());
    let updater = RecordingUpdater {
        calls: Mutex::new(Vec::new()),
        fail_team: Some("team-c".to_owned()),
    };
    let (logs, _guard) = capture_logs();
    update_default_team_budgets(Some(&no_budget), &master_key, &updater)
        .await
        .unwrap();
    assert!(logs.contents().contains(
        "Error updating team team-c with team member budget None with error: team missing, skipping.."
    ));
}
