#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_db::database_diagnostics::level::LogLevel;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{
    DEFAULT_CONCURRENCY, DEFAULT_EVENT_TIMEOUT, DEFAULT_MAX_QUEUE_SIZE, LogLevelError, level_number,
};

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load_error(environment: &[(&str, &str)]) -> ConfigError {
    ConfigLoader {
        lookup: lookup(environment),
    }
    .load("{}")
    .unwrap_err()
}

#[rstest]
#[case::defaults("{}", &[])]
#[case::list(
    "litellm_settings: {service_callback: [datadog, otel]}",
    &["datadog", "otel"]
)]
#[case::empty_list("litellm_settings: {service_callback: []}", &[])]
#[case::null("litellm_settings: {service_callback: null}", &[])]
fn service_callback_names_tests_behavior(#[case] yaml: &str, #[case] names: &[&str]) {
    let settings = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap()
    .sections
    .service_callback;
    assert_eq!(
        settings.service_callbacks,
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );
}

// LiteLLM declares service_callback as a list; a bare string is a value outside that type
#[test]
fn service_callback_bare_string_tests_behavior() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings: {service_callback: datadog}")
    .expect_err("a bare string is not the declared list");
    assert!(matches!(error, ConfigError::Yaml { .. }));
}

#[test]
fn service_callback_untouched_fields_keep_the_defaults_tests_behavior() {
    let settings = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("{}")
    .unwrap()
    .sections
    .service_callback;
    assert_eq!(settings, super::Settings::default());
}

#[rstest]
#[case::environment_reference(
    "litellm_settings: {service_callback: [os.environ/SERVICE]}",
    &[("SERVICE", "datadog")],
    Some(&["datadog"][..])
)]
#[case::missing_environment(
    "litellm_settings: {service_callback: [os.environ/ABSENT]}",
    &[],
    None
)]
fn service_callback_environment_tests_behavior(
    #[case] yaml: &str,
    #[case] environment: &[(&str, &str)],
    #[case] names: Option<&[&str]>,
) {
    let result = ConfigLoader {
        lookup: lookup(environment),
    }
    .load(yaml);
    match names {
        Some(expected) => assert_eq!(
            result.unwrap().sections.service_callback.service_callbacks,
            expected
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        ),
        None => assert!(matches!(
            result.unwrap_err(),
            ConfigError::MissingEnvironment { .. }
        )),
    }
}

#[rstest]
#[case::default(&[], LogLevel::Debug, 10_i32)]
#[case::lowercase(&[("LITELLM_LOG", "debug")], LogLevel::Debug, 10_i32)]
#[case::capitalized(&[("LITELLM_LOG", "Warn")], LogLevel::Warning, 30_i32)]
#[case::warning_name(&[("LITELLM_LOG", "warning")], LogLevel::Warning, 30_i32)]
#[case::fatal(&[("LITELLM_LOG", "fatal")], LogLevel::Critical, 50_i32)]
#[case::notset(&[("LITELLM_LOG", "NoTsEt")], LogLevel::NotSet, 0_i32)]
#[case::info(&[("LITELLM_LOG", "info")], LogLevel::Info, 20_i32)]
#[case::error(&[("LITELLM_LOG", "ERROR")], LogLevel::Error, 40_i32)]
#[case::empty_counts_as_unset(&[("LITELLM_LOG", "")], LogLevel::Debug, 10_i32)]
fn service_callback_log_level_tests_behavior(
    #[case] environment: &[(&str, &str)],
    #[case] level: LogLevel,
    #[case] number: i32,
) {
    let settings = ConfigLoader {
        lookup: lookup(environment),
    }
    .load("{}")
    .unwrap()
    .sections
    .service_callback;
    let resolved = settings.resolve_log_level().unwrap();
    assert_eq!(resolved, level);
    assert_eq!(level_number(resolved), number);
}

// the load keeps the raw value, so a bad LITELLM_LOG fails where the level is resolved, not here
#[rstest]
#[case::unknown("verbose")]
#[case::numeric("10")]
#[case::untrimmed(" info ")]
#[case::space_only(" ")]
fn service_callback_unknown_log_level_tests_behavior(#[case] value: &str) {
    let settings = ConfigLoader {
        lookup: lookup(&[("LITELLM_LOG", value)]),
    }
    .load("{}")
    .expect("the load carries the raw value")
    .sections
    .service_callback;
    let error = settings
        .resolve_log_level()
        .expect_err("an unknown name has no logging level");
    assert!(matches!(&error, LogLevelError::Unknown { .. }));
    assert_eq!(
        error.to_string(),
        format!("LITELLM_LOG={value:?} is not a logging level name")
    );
}

#[rstest]
#[case::default(&[], false)]
#[case::lowercase(&[("JSON_LOGS", "true")], true)]
#[case::uppercase(&[("JSON_LOGS", "TRUE")], true)]
#[case::mixed_case(&[("JSON_LOGS", "tRuE")], true)]
#[case::explicit_false(&[("JSON_LOGS", "false")], false)]
#[case::one(&[("JSON_LOGS", "1")], false)]
#[case::yes(&[("JSON_LOGS", "yes")], false)]
#[case::leading_space(&[("JSON_LOGS", " true")], false)]
#[case::trailing_space(&[("JSON_LOGS", "true ")], false)]
#[case::empty(&[("JSON_LOGS", "")], false)]
fn service_callback_json_logs_tests_behavior(
    #[case] environment: &[(&str, &str)],
    #[case] json: bool,
) {
    let settings = ConfigLoader {
        lookup: lookup(environment),
    }
    .load("{}")
    .unwrap()
    .sections
    .service_callback;
    assert_eq!(settings.json_logs, json);
}

#[rstest]
#[case::defaults(&[], 50_000, 100, Duration::from_secs(20))]
#[case::values(
    &[
        ("LOGGING_WORKER_MAX_QUEUE_SIZE", "10"),
        ("LOGGING_WORKER_CONCURRENCY", "4"),
        ("LOGGING_WORKER_MAX_TIME_PER_COROUTINE", "2.5"),
    ],
    10,
    4,
    Duration::from_millis(2500)
)]
#[case::empty_values(
    &[
        ("LOGGING_WORKER_MAX_QUEUE_SIZE", ""),
        ("LOGGING_WORKER_CONCURRENCY", ""),
        ("LOGGING_WORKER_MAX_TIME_PER_COROUTINE", ""),
    ],
    DEFAULT_MAX_QUEUE_SIZE,
    DEFAULT_CONCURRENCY,
    DEFAULT_EVENT_TIMEOUT
)]
#[case::trimmed(
    &[
        ("LOGGING_WORKER_MAX_QUEUE_SIZE", " 10 "),
        ("LOGGING_WORKER_CONCURRENCY", " 4 "),
        ("LOGGING_WORKER_MAX_TIME_PER_COROUTINE", " 2 "),
    ],
    10,
    4,
    Duration::from_secs(2)
)]
fn service_callback_worker_limits_tests_behavior(
    #[case] environment: &[(&str, &str)],
    #[case] queue: usize,
    #[case] concurrency: usize,
    #[case] timeout: Duration,
) {
    let settings = ConfigLoader {
        lookup: lookup(environment),
    }
    .load("{}")
    .unwrap()
    .sections
    .service_callback;
    assert_eq!(settings.max_queue_size, queue);
    assert_eq!(settings.concurrency, concurrency);
    assert_eq!(settings.event_timeout, timeout);
}

#[rstest]
#[case::queue_text(&[("LOGGING_WORKER_MAX_QUEUE_SIZE", "many")])]
#[case::queue_negative(&[("LOGGING_WORKER_MAX_QUEUE_SIZE", "-1")])]
#[case::concurrency_text(&[("LOGGING_WORKER_CONCURRENCY", "lots")])]
#[case::timeout_text(&[("LOGGING_WORKER_MAX_TIME_PER_COROUTINE", "soon")])]
#[case::timeout_negative(&[("LOGGING_WORKER_MAX_TIME_PER_COROUTINE", "-2")])]
fn service_callback_invalid_worker_limits_tests_behavior(#[case] environment: &[(&str, &str)]) {
    assert!(
        matches!(
            load_error(environment),
            ConfigError::Environment { .. } | ConfigError::InvalidScalar { .. }
        ),
        "the load accepted a worker limit"
    );
}

#[test]
fn service_callback_defaults_match_litellm_constants() {
    assert_eq!(DEFAULT_MAX_QUEUE_SIZE, 50_000);
    assert_eq!(DEFAULT_CONCURRENCY, 100);
    assert_eq!(DEFAULT_EVENT_TIMEOUT, Duration::from_secs(20));
}
