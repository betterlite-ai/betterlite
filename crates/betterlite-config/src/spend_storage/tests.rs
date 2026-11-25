#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_db::spend_flush::monitor::{DEFAULT_POLL_INTERVAL, DEFAULT_QUEUE_SIZE_THRESHOLD};
use betterlite_db::spend_partitions::calendar::PartitionInterval;
use betterlite_db::spend_retention::batches::settings::{
    CleanupBounds, DurationParser, Retention, SettingValue,
};
use rstest::rstest;
use rust_decimal::Decimal;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::monitor::{ENV_QUEUE_POLL_INTERVAL, ENV_QUEUE_SIZE_THRESHOLD};
use super::queue::{DEFAULT_QUEUE_MAX_BYTES, ENV_SPEND_LOG_QUEUE_MAX_BYTES, ENV_SPEND_LOGS_URL};
use super::retention::{
    ENV_BATCH_SIZE, ENV_BATCH_TIMEOUT, ENV_COUNT_CAP, ENV_FAILURE_BACKOFF, ENV_MAX_FAILURES,
    ENV_RUN_BUDGET, ENV_RUN_LOOPS,
};
use super::{
    DEFAULT_DEQUEUE_COUNT, DEFAULT_MAX_PROMPT_LENGTH, DEFAULT_PRECREATE_AHEAD, ENV_DEQUEUE_COUNT,
    ENV_PARTITION_INTERVAL, ENV_PRECREATE_AHEAD, MissingSessionId, Settings, SpendUpdates,
    decode_spend_storage_policy, load_spend_storage_policy, runtime_dequeue_count,
    runtime_max_prompt_length,
};
use crate::loader::ConfigLoader;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(source: &str, pairs: &[(&str, &str)]) -> Settings {
    load_spend_storage_policy(source, &environment(pairs)).unwrap()
}

#[test]
fn loads_spend_storage_policy() {
    let initial = load(
        "general_settings:\n  store_prompts_in_spend_logs: false\n  missing_session_id: omit\n",
        &[("MAX_STRING_LENGTH_PROMPT_IN_DB", "100")],
    );
    assert!(!initial.store_prompts_in_spend_logs);
    assert_eq!(initial.max_string_length_prompt_in_db, 100);
    assert_eq!(initial.missing_session_id, Some(MissingSessionId::Omit));
    assert_eq!(
        runtime_max_prompt_length(initial.max_string_length_prompt_in_db, Some("200")),
        200
    );
    assert_eq!(
        runtime_max_prompt_length(initial.max_string_length_prompt_in_db, None),
        100
    );
    assert_eq!(
        runtime_max_prompt_length(initial.max_string_length_prompt_in_db, Some("not-a-number")),
        100
    );
    assert_eq!(
        runtime_max_prompt_length(initial.max_string_length_prompt_in_db, Some("")),
        100
    );
    let defaults = load("general_settings: {}\n", &[]);
    assert_eq!(
        defaults.max_string_length_prompt_in_db,
        DEFAULT_MAX_PROMPT_LENGTH
    );
    assert_eq!(defaults.missing_session_id, None);
    assert_eq!(defaults.spend_updates, SpendUpdates::Allowed);
}

#[test]
fn loads_redis_spend_settings() {
    let defaults = load("general_settings: {}\n", &[]);
    assert!(!defaults.use_redis_transaction_buffer);
    assert_eq!(
        defaults.max_redis_buffer_dequeue_count,
        DEFAULT_DEQUEUE_COUNT
    );
    let enabled = load(
        "general_settings:\n  use_redis_transaction_buffer: true\n",
        &[(ENV_DEQUEUE_COUNT, "250")],
    );
    assert!(enabled.use_redis_transaction_buffer);
    assert_eq!(enabled.max_redis_buffer_dequeue_count, 250);
    let initial = enabled.max_redis_buffer_dequeue_count;
    assert_eq!(runtime_dequeue_count(initial, Some("10")), 10);
    assert_eq!(runtime_dequeue_count(initial, None), 250);
    assert_eq!(runtime_dequeue_count(initial, Some("")), 250);
    assert_eq!(runtime_dequeue_count(initial, Some("not-a-number")), 250);
}

#[test]
fn loads_spend_summary_budget() {
    let configured = load("litellm_settings:\n  max_budget: 100.5\n", &[]);
    assert_eq!(configured.max_budget, Decimal::new(1005, 1));
    let from_environment = load(
        "litellm_settings:\n  max_budget: os.environ/PROXY_MAX_BUDGET\n",
        &[("PROXY_MAX_BUDGET", "12.25")],
    );
    assert_eq!(from_environment.max_budget, Decimal::new(1225, 2));
    let defaults = load("general_settings: {}\n", &[]);
    assert_eq!(defaults.max_budget, Decimal::ZERO);
    let error = load_spend_storage_policy(
        "litellm_settings:\n  max_budget: not-a-number\n",
        &environment(&[]),
    )
    .unwrap_err();
    assert!(error.to_string().contains("litellm_settings.max_budget"));
}

#[test]
fn loads_spend_writer_settings() {
    let defaults = load("general_settings: {}\n", &[]);
    assert_eq!(defaults.disable_spend_logs, Some(false));
    assert_eq!(defaults.max_budget, Decimal::ZERO);
    let explicit_null = load("general_settings:\n  disable_spend_logs: null\n", &[]);
    assert_eq!(explicit_null.disable_spend_logs, None);
    let enabled = load(
        "general_settings:\n  disable_spend_logs: true\nlitellm_settings:\n  max_budget: 25.5\n",
        &[],
    );
    assert_eq!(enabled.disable_spend_logs, Some(true));
    assert_eq!(enabled.max_budget, Decimal::new(255, 1));
    let disabled = load("general_settings:\n  disable_spend_logs: false\n", &[]);
    assert_eq!(disabled.disable_spend_logs, Some(false));
    let error = load_spend_storage_policy(
        "general_settings:\n  disable_spend_logs: yes-please\n",
        &environment(&[]),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("general_settings.disable_spend_logs")
    );
}

#[rstest]
#[case::boolean_true("true", SpendUpdates::Disabled)]
#[case::boolean_false("false", SpendUpdates::Allowed)]
#[case::string_true("\"true\"", SpendUpdates::Allowed)]
#[case::number_one("1", SpendUpdates::Allowed)]
#[case::number_zero("0", SpendUpdates::Allowed)]
#[case::null("null", SpendUpdates::Allowed)]
fn loads_spend_update_flag(#[case] value: &str, #[case] expected: SpendUpdates) {
    let source = format!("general_settings:\n  disable_spend_updates: {value}\n");
    let settings = load(&source, &[]);
    assert_eq!(settings.spend_updates, expected);
}

#[rstest]
#[case::boolean_true("true", true)]
#[case::boolean_false("false", false)]
#[case::string_true("\"true\"", true)]
#[case::string_padded_upper("\" TRUE \"", true)]
#[case::string_false("\"false\"", false)]
#[case::unrecognized_string("\"yes\"", false)]
#[case::null("null", false)]
fn loads_redis_transaction_buffer_policy(#[case] value: &str, #[case] expected: bool) {
    let source = format!("general_settings:\n  use_redis_transaction_buffer: {value}\n");
    assert_eq!(load(&source, &[]).use_redis_transaction_buffer, expected);
}

#[rstest]
#[case::number("1")]
#[case::list("[true]")]
#[case::map("{}")]
fn rejects_a_redis_transaction_buffer_that_is_not_a_flag(#[case] value: &str) {
    let source = format!("general_settings:\n  use_redis_transaction_buffer: {value}\n");
    let error = load_spend_storage_policy(&source, &environment(&[])).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("general_settings.use_redis_transaction_buffer")
    );
}

#[rstest]
#[case::zero("0", 0)]
#[case::negative("-5", -5)]
#[case::surrounding_space(" 42 ", 42)]
fn keeps_zero_and_negative_redis_dequeue_counts(#[case] value: &str, #[case] expected: i64) {
    let settings = load("general_settings: {}\n", &[(ENV_DEQUEUE_COUNT, value)]);
    assert_eq!(settings.max_redis_buffer_dequeue_count, expected);
}

#[rstest]
#[case::text("many")]
#[case::fraction("20.5")]
#[case::python_digit_group("1_0")]
fn a_malformed_redis_dequeue_count_fails_loading(#[case] value: &str) {
    let source = "general_settings: {}\n";
    let lookup = environment(&[(ENV_DEQUEUE_COUNT, value)]);
    let error = load_spend_storage_policy(source, &lookup).unwrap_err();
    assert!(error.to_string().contains(ENV_DEQUEUE_COUNT));
}

// B2322's spend_reseed section owns the variable: unset keeps 10000, and int() rejects empty text
const LOCK_LIMIT_VARIABLE: &str = "SPEND_COUNTER_RESEED_LOCKS_MAX_SIZE";

#[rstest]
#[case::unset(&[], Some(10_000))]
#[case::configured(&[(LOCK_LIMIT_VARIABLE, "16")], Some(16))]
#[case::empty_text(&[(LOCK_LIMIT_VARIABLE, "")], None)]
#[case::spaces(&[(LOCK_LIMIT_VARIABLE, "  ")], None)]
#[case::text(&[(LOCK_LIMIT_VARIABLE, "many")], None)]
fn loads_reseed_lock_limit(#[case] pairs: &[(&str, &str)], #[case] expected: Option<i64>) {
    let outcome = ConfigLoader {
        lookup: environment(pairs),
    }
    .load("")
    .map(|config| config.sections.spend_reseed.locks_max_size);
    match (outcome, expected) {
        (Ok(bound), Some(wanted)) => assert_eq!(bound, wanted),
        (Err(error), None) => assert!(error.to_string().contains(LOCK_LIMIT_VARIABLE)),
        (Ok(bound), None) => panic!("the lock limit load accepted {bound}"),
        (Err(error), Some(wanted)) => panic!("the lock limit load failed for {wanted}: {error}"),
    }
}

#[rstest]
#[case::config_true("store_prompts_in_spend_logs: true", None, true)]
#[case::config_false("store_prompts_in_spend_logs: false", None, false)]
#[case::config_string_true("store_prompts_in_spend_logs: \"TRUE\"", None, true)]
#[case::config_string_false("store_prompts_in_spend_logs: \"true-ish\"", None, false)]
#[case::config_number("store_prompts_in_spend_logs: 1", None, false)]
#[case::env_true("store_prompts_in_spend_logs: false", Some("true"), true)]
#[case::env_upper("store_prompts_in_spend_logs: false", Some("TRUE"), true)]
#[case::env_yes("store_prompts_in_spend_logs: false", Some("yes"), false)]
#[case::env_false("store_prompts_in_spend_logs: true", Some("false"), true)]
fn resolves_prompts_storage_from_config_or_environment(
    #[case] setting: &str,
    #[case] store_prompts: Option<&str>,
    #[case] expected: bool,
) {
    let source = format!("general_settings:\n  {setting}\n");
    let pairs: Vec<(&str, &str)> = store_prompts
        .map(|value| vec![("STORE_PROMPTS_IN_SPEND_LOGS", value)])
        .unwrap_or_default();
    assert_eq!(load(&source, &pairs).store_prompts_in_spend_logs, expected);
}

#[rstest]
#[case::absent("", None)]
#[case::null("missing_session_id: null", None)]
#[case::generate("missing_session_id: generate", Some(MissingSessionId::Generate))]
#[case::reject("missing_session_id: reject", Some(MissingSessionId::Reject))]
#[case::omit("missing_session_id: omit", Some(MissingSessionId::Omit))]
fn parses_missing_session_policy(#[case] line: &str, #[case] expected: Option<MissingSessionId>) {
    let source = format!("general_settings:\n  {line}\n");
    let settings = load(&source, &[]);
    assert_eq!(settings.missing_session_id, expected);
}

#[rstest]
#[case::unknown("ignore")]
#[case::wrong_case("Omit")]
#[case::number("5")]
fn an_unknown_missing_session_policy_fails_loading(#[case] value: &str) {
    let source = format!("general_settings:\n  missing_session_id: {value}\n");
    let error = load_spend_storage_policy(&source, &environment(&[])).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("general_settings.missing_session_id")
    );
}

#[rstest]
#[case::text("not-a-number")]
#[case::fraction("20.5")]
#[case::python_digit_group("1_0")]
fn a_malformed_initial_prompt_length_fails_loading(#[case] value: &str) {
    let source = "general_settings: {}\n";
    let lookup = environment(&[("MAX_STRING_LENGTH_PROMPT_IN_DB", value)]);
    load_spend_storage_policy(source, &lookup).unwrap_err();
}

#[rstest]
#[case::zero("0", 0)]
#[case::negative("-5", -5)]
#[case::surrounding_space(" 42 ", 42)]
fn keeps_zero_and_negative_prompt_limits(#[case] value: &str, #[case] expected: i64) {
    let source = "general_settings: {}\n";
    let lookup = environment(&[("MAX_STRING_LENGTH_PROMPT_IN_DB", value)]);
    assert_eq!(
        load_spend_storage_policy(source, &lookup)
            .unwrap()
            .max_string_length_prompt_in_db,
        expected
    );
}

#[rstest]
fn replays_the_spend_storage_policy_corpus(
    #[files("fuzz/corpus/spend_storage_policy/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let _policy = decode_spend_storage_policy(&bytes);
}

#[test]
fn decodes_a_stored_policy_row() {
    let decoded = decode_spend_storage_policy(
        br#"{"general_settings":{"store_prompts_in_spend_logs":"true","disable_spend_updates":true},"litellm_settings":{"max_budget":7.5},"environment":{"MAX_STRING_LENGTH_PROMPT_IN_DB":"12"}}"#,
    )
    .unwrap();
    assert!(decoded.store_prompts_in_spend_logs);
    assert_eq!(decoded.spend_updates, SpendUpdates::Disabled);
    assert_eq!(decoded.max_string_length_prompt_in_db, 12);
    assert_eq!(decoded.max_budget, Decimal::new(75, 1));
    decode_spend_storage_policy(b"not json").unwrap_err();
}

#[rstest]
#[case::integer("100", Decimal::from(100_u8))]
#[case::float("7.5", Decimal::new(75, 1))]
#[case::text("\"12.25\"", Decimal::new(1225, 2))]
fn decodes_a_stored_budget(#[case] configured: &str, #[case] expected: Decimal) {
    let row = format!(r#"{{"litellm_settings":{{"max_budget":{configured}}},"environment":{{}}}}"#);
    let decoded = decode_spend_storage_policy(row.as_bytes()).unwrap();
    assert_eq!(decoded.max_budget, expected);
}

#[rstest]
#[case::boolean("true")]
#[case::array("[1]")]
#[case::object("{}")]
fn rejects_a_stored_budget_that_is_not_a_number(#[case] configured: &str) {
    let row = format!(r#"{{"litellm_settings":{{"max_budget":{configured}}},"environment":{{}}}}"#);
    let error = decode_spend_storage_policy(row.as_bytes()).unwrap_err();
    assert!(error.to_string().contains("litellm_settings.max_budget"));
}

#[rstest]
#[case::defaults("litellm_settings: {}\n", None, None)]
#[case::null_values(
    "litellm_settings:\n  disable_end_user_cost_tracking: null\n  enable_end_user_cost_tracking_prometheus_only: null\n",
    None,
    None
)]
#[case::disable_true(
    "litellm_settings:\n  disable_end_user_cost_tracking: true\n",
    Some(true),
    None
)]
#[case::disable_false(
    "litellm_settings:\n  disable_end_user_cost_tracking: false\n",
    Some(false),
    None
)]
#[case::prometheus_true(
    "litellm_settings:\n  enable_end_user_cost_tracking_prometheus_only: true\n",
    None,
    Some(true)
)]
#[case::prometheus_false(
    "litellm_settings:\n  enable_end_user_cost_tracking_prometheus_only: false\n",
    None,
    Some(false)
)]
#[case::both_set(
    "litellm_settings:\n  disable_end_user_cost_tracking: false\n  enable_end_user_cost_tracking_prometheus_only: true\n",
    Some(false),
    Some(true)
)]
fn loads_end_user_spend_settings(
    #[case] source: &str,
    #[case] disable: Option<bool>,
    #[case] prometheus_only: Option<bool>,
) {
    let settings = load(source, &[]);
    assert_eq!(settings.disable_end_user_cost_tracking, disable);
    assert_eq!(settings.enable_prometheus_only, prometheus_only);
}

#[rstest]
#[case::disable_string(
    "litellm_settings:\n  disable_end_user_cost_tracking: \"true\"\n",
    "litellm_settings.disable_end_user_cost_tracking"
)]
#[case::disable_number(
    "litellm_settings:\n  disable_end_user_cost_tracking: 1\n",
    "litellm_settings.disable_end_user_cost_tracking"
)]
#[case::prometheus_string(
    "litellm_settings:\n  enable_end_user_cost_tracking_prometheus_only: \"true\"\n",
    "litellm_settings.enable_end_user_cost_tracking_prometheus_only"
)]
#[case::prometheus_list(
    "litellm_settings:\n  enable_end_user_cost_tracking_prometheus_only: [true]\n",
    "litellm_settings.enable_end_user_cost_tracking_prometheus_only"
)]
fn loads_end_user_spend_settings_rejects_a_wrong_type(#[case] source: &str, #[case] path: &str) {
    let error = load_spend_storage_policy(source, &environment(&[])).unwrap_err();
    assert!(error.to_string().contains(path));
}

#[test]
fn loads_end_user_spend_settings_from_a_stored_policy_row() {
    let decoded = decode_spend_storage_policy(
        br#"{"litellm_settings":{"disable_end_user_cost_tracking":true,"enable_end_user_cost_tracking_prometheus_only":false},"environment":{}}"#,
    )
    .unwrap();
    assert_eq!(decoded.disable_end_user_cost_tracking, Some(true));
    assert_eq!(decoded.enable_prometheus_only, Some(false));
}

#[derive(Clone)]
struct EventCapture(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for EventCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut fields = FieldCapture::default();
        event.record(&mut fields);
        if let Ok(mut events) = self.0.lock() {
            events.push(fields.0.join(" "));
        }
    }
}

#[derive(Default)]
struct FieldCapture(Vec<String>);

impl Visit for FieldCapture {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push(format!("{}={value}", field.name()));
    }
}

fn partition_settings(pairs: &[(&str, &str)]) -> (Settings, Vec<String>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&events)));
    let settings = with_default(subscriber, || load("general_settings: {}\n", pairs));
    let captured = events.lock().unwrap().clone();
    (settings, captured)
}

#[rstest]
#[case::defaults(None, None, PartitionInterval::Day, DEFAULT_PRECREATE_AHEAD, 0)]
#[case::day(Some("day"), Some("0"), PartitionInterval::Day, 0, 0)]
#[case::week(Some("week"), Some("-2"), PartitionInterval::Week, -2, 0)]
#[case::month(
    Some("month"),
    None,
    PartitionInterval::Month,
    DEFAULT_PRECREATE_AHEAD,
    0
)]
#[case::invalid_interval(
    Some("weekly"),
    None,
    PartitionInterval::Day,
    DEFAULT_PRECREATE_AHEAD,
    1
)]
#[case::upper_case(Some("WEEK"), None, PartitionInterval::Day, DEFAULT_PRECREATE_AHEAD, 1)]
#[case::empty_values(Some(""), Some(""), PartitionInterval::Day, DEFAULT_PRECREATE_AHEAD, 0)]
fn loads_spend_partition_settings(
    #[case] interval: Option<&str>,
    #[case] ahead: Option<&str>,
    #[case] expected_interval: PartitionInterval,
    #[case] expected_ahead: i64,
    #[case] warnings: usize,
) {
    let mut pairs = Vec::new();
    if let Some(value) = interval {
        pairs.push((ENV_PARTITION_INTERVAL, value));
    }
    if let Some(value) = ahead {
        pairs.push((ENV_PRECREATE_AHEAD, value));
    }
    let (settings, events) = partition_settings(&pairs);
    assert_eq!(settings.partition_interval, expected_interval);
    assert_eq!(settings.precreate_ahead, expected_ahead);
    let warned = events
        .iter()
        .filter(|event| event.contains(ENV_PARTITION_INTERVAL))
        .count();
    assert_eq!(warned, warnings);
}

#[rstest]
#[case::text("many")]
#[case::fraction("20.5")]
#[case::python_digit_group("1_0")]
fn loads_spend_partition_settings_rejects_a_malformed_ahead(#[case] value: &str) {
    let error = load_spend_storage_policy(
        "general_settings: {}\n",
        &environment(&[(ENV_PRECREATE_AHEAD, value)]),
    )
    .unwrap_err();
    assert!(error.to_string().contains(ENV_PRECREATE_AHEAD));
}

#[test]
fn loads_spend_partition_settings_from_a_stored_row() {
    let decoded = decode_spend_storage_policy(
        br#"{"general_settings":{},"litellm_settings":{},"environment":{"SPEND_LOG_PARTITION_INTERVAL":"month","SPEND_LOG_PARTITION_PRECREATE_AHEAD":"3"}}"#,
    )
    .unwrap();
    assert_eq!(decoded.partition_interval, PartitionInterval::Month);
    assert_eq!(decoded.precreate_ahead, 3);
}

struct Durations;

impl DurationParser for Durations {
    type Failure = String;

    fn duration_seconds(&self, spec: &str) -> Result<i64, String> {
        match spec {
            "10m" => Ok(600),
            "90s" => Ok(90),
            "0s" => Ok(0),
            "12h" => Ok(43_200),
            "30d" => Ok(2_592_000),
            "7d" => Ok(604_800),
            other => Err(format!("unsupported duration {other}")),
        }
    }
}

const SHIPPED_BOUNDS: CleanupBounds = CleanupBounds {
    batch_size: 1_000,
    max_batches: 500,
    run_budget: Duration::from_secs(300),
    batch_timeout: Duration::from_secs(30),
    max_consecutive_failures: 3,
    failure_backoff: Duration::from_millis(500),
    remaining_count_cap: 100_000,
};

const OVERRIDDEN_BOUNDS: CleanupBounds = CleanupBounds {
    batch_size: 250,
    max_batches: 40,
    run_budget: Duration::from_secs(600),
    batch_timeout: Duration::from_secs(90),
    ..SHIPPED_BOUNDS
};

const ENVIRONMENT_BOUNDS: CleanupBounds = CleanupBounds {
    batch_size: 50,
    max_batches: 7,
    run_budget: Duration::from_millis(12_500),
    batch_timeout: Duration::ZERO,
    max_consecutive_failures: 0,
    failure_backoff: Duration::from_millis(250),
    remaining_count_cap: 10,
};

const OVERRIDDEN_ENVIRONMENT: CleanupBounds = CleanupBounds {
    batch_size: 250,
    max_batches: 40,
    run_budget: Duration::from_secs(600),
    batch_timeout: Duration::from_secs(90),
    ..ENVIRONMENT_BOUNDS
};

const EVERY_KEY: &str = "general_settings:
  maximum_spend_logs_cleanup_batch_size: 250
  maximum_spend_logs_cleanup_max_batches: '40'
  maximum_spend_logs_cleanup_run_budget: 10m
  maximum_spend_logs_cleanup_batch_timeout: 90s
  maximum_spend_logs_retention_period: 30d
  maximum_autorouter_session_retention_period: 12h
  maximum_health_check_retention_period: 7
  maximum_daily_tag_spend_retention_period: 90s
";

const CLEARED_KEYS: &str = "general_settings:
  maximum_spend_logs_cleanup_batch_size: null
  maximum_spend_logs_cleanup_max_batches: null
  maximum_spend_logs_cleanup_run_budget: null
  maximum_spend_logs_cleanup_batch_timeout: null
  maximum_spend_logs_retention_period: null
";

const UNUSABLE_KEYS: &str = "general_settings:
  maximum_spend_logs_cleanup_batch_size: 0
  maximum_spend_logs_cleanup_max_batches: many
  maximum_spend_logs_cleanup_run_budget: 0s
  maximum_spend_logs_cleanup_batch_timeout: soon
  maximum_spend_logs_retention_period: forever
";

const CLEANUP_ENVIRONMENT: [(&str, &str); 7] = [
    (ENV_BATCH_SIZE, "50"),
    (ENV_RUN_LOOPS, " 7 "),
    (ENV_RUN_BUDGET, "12.5"),
    (ENV_BATCH_TIMEOUT, "-1"),
    (ENV_MAX_FAILURES, "0"),
    (ENV_FAILURE_BACKOFF, "0.25"),
    (ENV_COUNT_CAP, "10"),
];

const EMPTY_ENVIRONMENT: [(&str, &str); 7] = [
    (ENV_BATCH_SIZE, ""),
    (ENV_RUN_LOOPS, ""),
    (ENV_RUN_BUDGET, ""),
    (ENV_BATCH_TIMEOUT, ""),
    (ENV_MAX_FAILURES, ""),
    (ENV_FAILURE_BACKOFF, ""),
    (ENV_COUNT_CAP, ""),
];

const RETENTIONS: [Retention; 4] = [
    Retention::SpendLogs,
    Retention::AutorouterSessions,
    Retention::HealthChecks,
    Retention::DailyTagSpend,
];

const EVERY_RETENTION: [Option<i64>; 4] = [Some(2_592_000), Some(43_200), Some(604_800), Some(90)];

#[rstest]
#[case::shipped_defaults("general_settings: {}\n", &[], SHIPPED_BOUNDS, [None; 4])]
#[case::every_key(EVERY_KEY, &[], OVERRIDDEN_BOUNDS, EVERY_RETENTION)]
#[case::reset_to_default(CLEARED_KEYS, &[], SHIPPED_BOUNDS, [None; 4])]
#[case::environment_defaults(
    "general_settings: {}\n",
    &CLEANUP_ENVIRONMENT,
    ENVIRONMENT_BOUNDS,
    [None; 4]
)]
#[case::unusable_overrides_use_the_environment(
    UNUSABLE_KEYS,
    &CLEANUP_ENVIRONMENT,
    ENVIRONMENT_BOUNDS,
    [None; 4]
)]
#[case::overrides_beat_the_environment(
    EVERY_KEY,
    &CLEANUP_ENVIRONMENT,
    OVERRIDDEN_ENVIRONMENT,
    EVERY_RETENTION
)]
#[case::empty_environment("general_settings: {}\n", &EMPTY_ENVIRONMENT, SHIPPED_BOUNDS, [None; 4])]
fn loads_spend_retention_settings(
    #[case] source: &str,
    #[case] pairs: &[(&str, &str)],
    #[case] expected: CleanupBounds,
    #[case] retention: [Option<i64>; 4],
) {
    let settings = load(source, pairs);
    assert_eq!(settings.retention.bounds(&Durations), expected);
    let read = RETENTIONS.map(|group| settings.retention.retention_seconds(group, &Durations));
    assert_eq!(read, retention);
}

#[test]
fn loads_spend_retention_settings_live_after_a_reset() {
    let configured = load(EVERY_KEY, &[]);
    assert_eq!(configured.retention.bounds(&Durations), OVERRIDDEN_BOUNDS);
    let cleared = load(CLEARED_KEYS, &[]);
    assert_eq!(cleared.retention.bounds(&Durations), SHIPPED_BOUNDS);
    assert_eq!(
        cleared
            .retention
            .retention_seconds(Retention::SpendLogs, &Durations),
        None
    );
}

#[test]
fn loads_retention_partition_switch() {
    let defaults = load("general_settings: {}\n", &[]);
    assert_eq!(defaults.retention.general.use_spend_logs_partitioning, None);
    let enabled = load(
        "general_settings:\n  use_spend_logs_partitioning: true\n  maximum_spend_logs_retention_period: 7d\n",
        &[],
    );
    assert_eq!(
        enabled.retention.general.use_spend_logs_partitioning,
        Some(SettingValue::Flag(true))
    );
    assert_eq!(
        enabled
            .retention
            .retention_seconds(Retention::SpendLogs, &Durations),
        Some(604_800)
    );
    let disabled = load(
        "general_settings:\n  use_spend_logs_partitioning: false\n",
        &[],
    );
    assert_eq!(
        disabled.retention.general.use_spend_logs_partitioning,
        Some(SettingValue::Flag(false))
    );
    let cleared = load(
        "general_settings:\n  use_spend_logs_partitioning: null\n",
        &[],
    );
    assert_eq!(cleared.retention.general.use_spend_logs_partitioning, None);
}

#[test]
fn a_non_boolean_partition_switch_fails_loading() {
    let error = load_spend_storage_policy(
        "general_settings:\n  use_spend_logs_partitioning: always\n",
        &environment(&[]),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("general_settings.use_spend_logs_partitioning")
    );
}

#[rstest]
#[case::fraction(ENV_BATCH_SIZE, "1.5")]
#[case::digit_group(ENV_RUN_LOOPS, "1_0")]
#[case::word(ENV_RUN_BUDGET, "soon")]
#[case::infinite(ENV_BATCH_TIMEOUT, "1e999")]
#[case::failures(ENV_MAX_FAILURES, "three")]
#[case::backoff(ENV_FAILURE_BACKOFF, "fast")]
#[case::cap(ENV_COUNT_CAP, "2.5")]
fn loads_spend_retention_settings_rejects_a_malformed_number(
    #[case] name: &str,
    #[case] value: &str,
) {
    let error = load_spend_storage_policy("general_settings: {}\n", &environment(&[(name, value)]))
        .unwrap_err();
    assert!(error.to_string().contains(name));
}

#[test]
fn loads_spend_retention_settings_from_a_stored_row() {
    let decoded = decode_spend_storage_policy(
        br#"{"general_settings":{"maximum_spend_logs_cleanup_batch_size":2.9,"maximum_spend_logs_retention_period":7},"litellm_settings":{},"environment":{"SPEND_LOG_CLEANUP_BATCH_SIZE":"40"}}"#,
    )
    .unwrap();
    assert_eq!(decoded.retention.defaults.batch_size, 40);
    assert_eq!(decoded.retention.bounds(&Durations).batch_size, 2);
    assert_eq!(
        decoded
            .retention
            .retention_seconds(Retention::SpendLogs, &Durations),
        Some(604_800)
    );
}

#[test]
fn loads_spend_log_storage_settings() {
    let defaults = load("general_settings: {}\n", &[]);
    assert_eq!(defaults.spend_log_queue_max_bytes, DEFAULT_QUEUE_MAX_BYTES);
    assert_eq!(defaults.spend_logs_url, None);
    let configured = load(
        "general_settings: {}\n",
        &[
            (ENV_SPEND_LOG_QUEUE_MAX_BYTES, "1024"),
            (ENV_SPEND_LOGS_URL, "https://spend.example/api"),
        ],
    );
    assert_eq!(configured.spend_log_queue_max_bytes, 1024);
    assert_eq!(
        configured.spend_logs_url.as_deref(),
        Some("https://spend.example/api")
    );
}

#[rstest]
#[case::zero("0", 1)]
#[case::negative("-5", 1)]
#[case::one("1", 1)]
#[case::surrounding_space(" 42 ", 42)]
fn clamps_the_spend_log_queue_byte_budget(#[case] value: &str, #[case] expected: usize) {
    let settings = load(
        "general_settings: {}\n",
        &[(ENV_SPEND_LOG_QUEUE_MAX_BYTES, value)],
    );
    assert_eq!(settings.spend_log_queue_max_bytes, expected);
}

#[rstest]
#[case::text("many")]
#[case::fraction("20.5")]
#[case::python_digit_group("1_0")]
fn a_malformed_spend_log_queue_byte_budget_fails_loading(#[case] value: &str) {
    let lookup = environment(&[(ENV_SPEND_LOG_QUEUE_MAX_BYTES, value)]);
    let error = load_spend_storage_policy("general_settings: {}\n", &lookup).unwrap_err();
    assert!(error.to_string().contains(ENV_SPEND_LOG_QUEUE_MAX_BYTES));
}

#[test]
fn treats_an_empty_spend_logs_url_as_unset() {
    let settings = load("general_settings: {}\n", &[(ENV_SPEND_LOGS_URL, "")]);
    assert_eq!(settings.spend_logs_url, None);
}

#[rstest]
#[case::absent("litellm_settings: {}\n", None)]
#[case::null("litellm_settings:\n  cold_storage_custom_logger: null\n", None)]
#[case::named(
    "litellm_settings:\n  cold_storage_custom_logger: s3_v2\n",
    Some("s3_v2")
)]
#[case::empty("litellm_settings:\n  cold_storage_custom_logger: \"\"\n", Some(""))]
fn loads_spend_cold_storage_logger(#[case] source: &str, #[case] expected: Option<&str>) {
    let settings = load(source, &[]);
    assert_eq!(settings.cold_storage_custom_logger.as_deref(), expected);
}

#[rstest]
#[case::boolean("true")]
#[case::number("3")]
#[case::list("[s3_v2]")]
#[case::map("{name: s3_v2}")]
fn loads_spend_cold_storage_only_from_a_name(#[case] value: &str) {
    let source = format!("litellm_settings:\n  cold_storage_custom_logger: {value}\n");
    let error = load_spend_storage_policy(&source, &environment(&[])).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("litellm_settings.cold_storage_custom_logger")
    );
}

#[test]
fn loads_spend_cold_storage_from_a_stored_policy_row() {
    let decoded = decode_spend_storage_policy(
        br#"{"litellm_settings":{"cold_storage_custom_logger":"gcs_bucket"}}"#,
    )
    .unwrap();
    assert_eq!(
        decoded.cold_storage_custom_logger.as_deref(),
        Some("gcs_bucket")
    );
}

#[test]
fn loads_spend_monitor_settings() {
    let defaults = load("general_settings: {}\n", &[]);
    assert_eq!(
        defaults.spend_log_monitor.queue_size_threshold,
        DEFAULT_QUEUE_SIZE_THRESHOLD
    );
    assert_eq!(
        defaults.spend_log_monitor.base_interval,
        DEFAULT_POLL_INTERVAL
    );

    let configured = load(
        "general_settings: {}\n",
        &[
            (ENV_QUEUE_SIZE_THRESHOLD, "250"),
            (ENV_QUEUE_POLL_INTERVAL, "0.5"),
        ],
    );
    assert_eq!(configured.spend_log_monitor.queue_size_threshold, 250);
    assert_eq!(
        configured.spend_log_monitor.base_interval,
        Duration::from_millis(500)
    );
}

#[test]
fn an_empty_spend_monitor_value_counts_as_unset() {
    let settings = load(
        "general_settings: {}\n",
        &[
            (ENV_QUEUE_SIZE_THRESHOLD, ""),
            (ENV_QUEUE_POLL_INTERVAL, ""),
        ],
    );
    assert_eq!(
        settings.spend_log_monitor.queue_size_threshold,
        DEFAULT_QUEUE_SIZE_THRESHOLD
    );
    assert_eq!(
        settings.spend_log_monitor.base_interval,
        DEFAULT_POLL_INTERVAL
    );
}

#[rstest]
#[case::zero("0")]
#[case::fractional_zero("0.0")]
#[case::negative("-3")]
fn a_nonpositive_spend_log_poll_interval_polls_at_once(#[case] value: &str) {
    let settings = load(
        "general_settings: {}\n",
        &[(ENV_QUEUE_POLL_INTERVAL, value)],
    );
    assert_eq!(settings.spend_log_monitor.base_interval, Duration::ZERO);
}

#[rstest]
#[case::infinite_overflow("1e999")]
#[case::not_a_number("nan")]
#[case::infinity("inf")]
fn a_nonfinite_spend_log_poll_interval_fails_loading(#[case] value: &str) {
    let error = load_spend_storage_policy(
        "general_settings: {}\n",
        &environment(&[(ENV_QUEUE_POLL_INTERVAL, value)]),
    )
    .unwrap_err();
    assert!(error.to_string().contains(ENV_QUEUE_POLL_INTERVAL));
}

#[test]
fn a_malformed_spend_log_queue_threshold_fails_loading() {
    let error = load_spend_storage_policy(
        "general_settings: {}\n",
        &environment(&[(ENV_QUEUE_SIZE_THRESHOLD, "many")]),
    )
    .unwrap_err();
    assert!(error.to_string().contains(ENV_QUEUE_SIZE_THRESHOLD));
}
