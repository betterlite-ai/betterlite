#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::database_diagnostics::level::LogLevel;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::worker_options_reload::{
    LogConfig, Settings, UvicornSettings, resolve_log_level,
};

fn load(environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("{}")
        .map(|config| config.sections.worker_options_reload)
}

fn loaded(environment: &[(&str, &str)]) -> Settings {
    load(environment).unwrap()
}

#[test]
fn an_empty_config_takes_the_litellm_worker_defaults() {
    let settings = loaded(&[]);
    assert_eq!(settings.worker_options, UvicornSettings::default());
    assert_eq!(settings.worker_options.host, "0.0.0.0");
    assert_eq!(settings.worker_options.port, 4000);
    assert!(!settings.worker_options.server_header);
    assert_eq!(settings.worker_options.log_config, LogConfig::Builtin);
    assert_eq!(settings.worker_options.timeout_keep_alive, None);
    assert_eq!(settings.worker_options.timeout_worker_healthcheck, None);
    assert_eq!(settings.worker_options.limit_max_requests, None);
    assert_eq!(settings.worker_options.limit_max_requests_jitter, None);
    assert_eq!(settings.worker_options.loop_type, None);
}

#[rstest]
#[case::address(&[("HOST", "127.0.0.1")], "127.0.0.1")]
#[case::wildcard(&[("HOST", "0.0.0.0")], "0.0.0.0")]
#[case::ipv6(&[("HOST", "::1")], "::1")]
fn host_comes_from_the_environment(#[case] environment: &[(&str, &str)], #[case] expected: &str) {
    assert_eq!(loaded(environment).worker_options.host, expected);
}

#[rstest]
#[case::eight_thousand(&[("PORT", "8000")], 8000)]
#[case::lowest(&[("PORT", "0")], 0)]
#[case::highest(&[("PORT", "65535")], 65535)]
fn port_comes_from_the_environment(#[case] environment: &[(&str, &str)], #[case] expected: u16) {
    assert_eq!(loaded(environment).worker_options.port, expected);
}

#[rstest]
#[case::twelve(&[("KEEPALIVE_TIMEOUT", "12")], Some(12))]
#[case::zero(&[("KEEPALIVE_TIMEOUT", "0")], Some(0))]
#[case::negative(&[("KEEPALIVE_TIMEOUT", "-3")], Some(-3))]
fn keepalive_timeout_comes_from_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<i64>,
) {
    assert_eq!(
        loaded(environment).worker_options.timeout_keep_alive,
        expected
    );
}

#[rstest]
#[case::zero("0", Some(0))]
#[case::positive("45", Some(45))]
#[case::negative("-1", Some(-1))]
fn timeout_worker_healthcheck_comes_from_the_environment(
    #[case] value: &str,
    #[case] expected: Option<i64>,
) {
    let settings = loaded(&[("TIMEOUT_WORKER_HEALTHCHECK", value)]);
    assert_eq!(settings.worker_options.timeout_worker_healthcheck, expected);
}

#[rstest]
#[case::zero("0", Some(0))]
#[case::positive("100", Some(100))]
#[case::negative("-5", Some(-5))]
fn max_requests_come_from_the_environment(#[case] value: &str, #[case] expected: Option<i64>) {
    let settings = loaded(&[("MAX_REQUESTS_BEFORE_RESTART", value)]);
    assert_eq!(settings.worker_options.limit_max_requests, expected);
}

#[rstest]
#[case::zero("0", Some(0))]
#[case::positive("10", Some(10))]
fn jitter_comes_from_the_environment(#[case] value: &str, #[case] expected: Option<i64>) {
    let settings = loaded(&[("MAX_REQUESTS_BEFORE_RESTART_JITTER", value)]);
    assert_eq!(settings.worker_options.limit_max_requests_jitter, expected);
}

#[rstest]
#[case::host(&[("HOST", "")])]
#[case::port(&[("PORT", "")])]
#[case::keepalive(&[("KEEPALIVE_TIMEOUT", "")])]
#[case::healthcheck(&[("TIMEOUT_WORKER_HEALTHCHECK", "")])]
#[case::max_requests(&[("MAX_REQUESTS_BEFORE_RESTART", "")])]
#[case::jitter(&[("MAX_REQUESTS_BEFORE_RESTART_JITTER", "")])]
fn empty_variables_count_as_unset(#[case] environment: &[(&str, &str)]) {
    assert_eq!(
        loaded(environment).worker_options,
        UvicornSettings::default()
    );
}

#[rstest]
#[case::keepalive(&[("KEEPALIVE_TIMEOUT", "soon")])]
#[case::port(&[("PORT", "70000")])]
#[case::healthcheck(&[("TIMEOUT_WORKER_HEALTHCHECK", "1.5")])]
#[case::max_requests(&[("MAX_REQUESTS_BEFORE_RESTART", "many")])]
fn malformed_worker_variables_fail_the_load(#[case] environment: &[(&str, &str)]) {
    assert!(matches!(
        load(environment),
        Err(ConfigError::Environment { .. })
    ));
}

#[rstest]
#[case::lowercase("true", LogConfig::Json)]
#[case::uppercase("TRUE", LogConfig::Json)]
#[case::mixed_case("tRuE", LogConfig::Json)]
#[case::explicit_false("false", LogConfig::Builtin)]
#[case::zero("0", LogConfig::Builtin)]
#[case::one("1", LogConfig::Builtin)]
#[case::yes("yes", LogConfig::Builtin)]
#[case::leading_space(" true", LogConfig::Builtin)]
#[case::trailing_space("true ", LogConfig::Builtin)]
fn json_logs_select_the_json_config_only_for_true(
    #[case] value: &str,
    #[case] expected: LogConfig,
) {
    assert_eq!(
        loaded(&[("JSON_LOGS", value)]).worker_options.log_config,
        expected
    );
}

#[rstest]
#[case::debug("debug", LogLevel::Debug)]
#[case::capitalized("Debug", LogLevel::Debug)]
#[case::info("info", LogLevel::Info)]
#[case::warn("Warn", LogLevel::Warning)]
#[case::warning_name("WARNING", LogLevel::Warning)]
#[case::error("error", LogLevel::Error)]
#[case::fatal("fatal", LogLevel::Critical)]
#[case::notset("NoTsEt", LogLevel::NotSet)]
fn lite_llm_log_selects_the_log_level(#[case] value: &str, #[case] expected: LogLevel) {
    assert_eq!(
        loaded(&[("LITELLM_LOG", value)]).worker_options.log_config,
        LogConfig::Level(expected)
    );
}

#[test]
fn json_logs_win_over_the_litellm_log_level() {
    let settings = loaded(&[("JSON_LOGS", "true"), ("LITELLM_LOG", "debug")]);
    assert_eq!(settings.worker_options.log_config, LogConfig::Json);
}

#[test]
fn an_unknown_log_level_fails_the_load() {
    assert!(matches!(
        load(&[("LITELLM_LOG", "verbose")]),
        Err(ConfigError::InvalidScalar {
            ref path,
            value: false
        }) if path == "LITELLM_LOG"
    ));
}

#[test]
fn an_unknown_log_level_fails_the_load_even_with_json_logs() {
    assert!(matches!(
        load(&[("JSON_LOGS", "true"), ("LITELLM_LOG", "verbose")]),
        Err(ConfigError::InvalidScalar {
            ref path,
            value: false
        }) if path == "LITELLM_LOG"
    ));
}

#[rstest]
#[case::python_name("DEBUG", LogLevel::Debug)]
#[case::lowercase("info", LogLevel::Info)]
#[case::warn_alias("WARN", LogLevel::Warning)]
#[case::critical_alias("critical", LogLevel::Critical)]
fn resolve_log_level_matches_pythons_logging_names(#[case] text: &str, #[case] expected: LogLevel) {
    assert_eq!(resolve_log_level(text).unwrap(), expected);
}

#[test]
fn resolve_log_level_rejects_a_name_python_logging_lacks() {
    assert!(matches!(
        resolve_log_level("verbose"),
        Err(ConfigError::InvalidScalar {
            ref path,
            value: false
        }) if path == "LITELLM_LOG"
    ));
}

#[test]
fn reload_starts_disabled_with_no_watches() {
    let settings = loaded(&[]);
    assert!(!settings.dev_reload.reload);
    assert!(settings.dev_reload.reload_dirs.is_empty());
    assert!(settings.dev_reload.reload_includes.is_empty());
    assert!(settings.dev_reload.extra_paths.is_empty());
    assert!(!settings.dev_reload.env_hot_reload);
}

#[rstest]
#[case::python_bool("True", true)]
#[case::lowercase("true", true)]
#[case::one("1", true)]
#[case::explicit_false("false", false)]
#[case::zero("0", false)]
fn dev_reload_reads_the_hot_reload_variable(#[case] value: &str, #[case] expected: bool) {
    assert_eq!(
        loaded(&[("LITELLM_DEV_ENV_HOT_RELOAD", value)])
            .dev_reload
            .env_hot_reload,
        expected
    );
}

#[test]
fn a_malformed_hot_reload_flag_fails_the_load() {
    assert!(matches!(
        load(&[("LITELLM_DEV_ENV_HOT_RELOAD", "sometimes")]),
        Err(ConfigError::Environment { .. })
    ));
}
