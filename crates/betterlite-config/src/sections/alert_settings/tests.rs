#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::alert_settings::Settings;
use crate::sections::alert_settings::types::{AlertType, DEFAULT_ALERT_TYPES, SlackAlertingArgs};
use crate::sections::alert_settings::webhooks::WebhookUrl;

fn loaded(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.alert_settings)
}

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    loaded(source, variables).expect("the alert settings load")
}

fn load_error(source: &str, variables: &[(&str, &str)]) -> ConfigError {
    loaded(source, variables).expect_err("the alert settings fail to load")
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the test decimal parses")
}

fn exposed(url: &WebhookUrl) -> Vec<String> {
    match url {
        WebhookUrl::Single(value) => vec![value.expose_secret().to_owned()],
        WebhookUrl::Multiple(values) => values
            .iter()
            .map(|value| value.expose_secret().to_owned())
            .collect(),
    }
}

#[test]
fn defaults_match_litellm_when_a_config_sets_no_alerting() {
    let loaded = settings("general_settings: {}", &[]);
    assert_eq!(loaded.alerting_args, Settings::default().alerting_args);
    assert!(loaded.alerting.is_empty());
    assert_eq!(
        loaded.alerting_threshold_seconds.to_bits(),
        300.0_f64.to_bits()
    );
    assert_eq!(loaded.alert_types, DEFAULT_ALERT_TYPES.to_vec());
    assert!(loaded.alert_to_webhook_url.is_empty());
    assert!(loaded.alert_type_config.is_empty());
}

#[test]
fn alerting_arg_defaults_match_litellm() {
    let expected = SlackAlertingArgs {
        daily_report_frequency: 43_200,
        report_check_interval: 300,
        budget_alert_ttl: 86_400,
        outage_alert_ttl: 60,
        region_outage_alert_ttl: 60,
        minor_outage_alert_threshold: 5,
        major_outage_alert_threshold: 10,
        max_outage_alert_list_size: 10,
        log_to_console: false,
        daily_user_spend_threshold: None,
        monthly_user_spend_threshold: None,
        spend_anomaly_multiplier: decimal("3"),
        spend_anomaly_baseline_days: 7,
        spend_anomaly_min_spend: decimal("10"),
        user_spend_check_interval: 3600,
    };
    assert_eq!(
        settings("general_settings: {}", &[]).alerting_args,
        expected
    );
}

#[test]
fn alerting_args_override_every_default() {
    let source = "general_settings:\n  alerting_args:\n    daily_report_frequency: 60\n    report_check_interval: 90\n    budget_alert_ttl: 120\n    outage_alert_ttl: 30\n    region_outage_alert_ttl: 45\n    minor_outage_alert_threshold: 2\n    major_outage_alert_threshold: 4\n    max_outage_alert_list_size: 3\n    log_to_console: true\n    daily_spend_per_user_threshold: 1.5\n    monthly_spend_per_user_threshold: 20\n    spend_anomaly_multiplier: 2.5\n    spend_anomaly_baseline_days: 14\n    spend_anomaly_min_spend: 5\n    user_spend_check_interval: 120";
    let expected = SlackAlertingArgs {
        daily_report_frequency: 60,
        report_check_interval: 90,
        budget_alert_ttl: 120,
        outage_alert_ttl: 30,
        region_outage_alert_ttl: 45,
        minor_outage_alert_threshold: 2,
        major_outage_alert_threshold: 4,
        max_outage_alert_list_size: 3,
        log_to_console: true,
        daily_user_spend_threshold: Some(decimal("1.5")),
        monthly_user_spend_threshold: Some(decimal("20")),
        spend_anomaly_multiplier: decimal("2.5"),
        spend_anomaly_baseline_days: 14,
        spend_anomaly_min_spend: decimal("5"),
        user_spend_check_interval: 120,
    };
    assert_eq!(settings(source, &[]).alerting_args, expected);
}

#[test]
fn null_alerting_args_take_the_defaults() {
    let loaded = settings("general_settings:\n  alerting_args: null", &[]);
    assert_eq!(loaded.alerting_args, Settings::default().alerting_args);
}

#[test]
fn slack_daily_report_frequency_comes_from_the_environment() {
    let loaded = settings(
        "general_settings: {}",
        &[("SLACK_DAILY_REPORT_FREQUENCY", "60")],
    );
    assert_eq!(loaded.alerting_args.daily_report_frequency, 60);
}

#[test]
fn an_explicit_daily_report_frequency_beats_the_environment() {
    let loaded = settings(
        "general_settings:\n  alerting_args:\n    daily_report_frequency: 120",
        &[("SLACK_DAILY_REPORT_FREQUENCY", "60")],
    );
    assert_eq!(loaded.alerting_args.daily_report_frequency, 120);
}

#[test]
fn an_empty_daily_report_frequency_variable_counts_as_unset() {
    let loaded = settings(
        "general_settings: {}",
        &[("SLACK_DAILY_REPORT_FREQUENCY", "")],
    );
    assert_eq!(loaded.alerting_args.daily_report_frequency, 43_200);
}

#[test]
fn a_non_numeric_daily_report_frequency_is_a_startup_error_naming_the_variable() {
    let error = load_error(
        "general_settings: {}",
        &[("SLACK_DAILY_REPORT_FREQUENCY", "soon")],
    );
    assert_eq!(error.category(), "environment");
    assert!(
        error.to_string().contains("SLACK_DAILY_REPORT_FREQUENCY"),
        "{error}"
    );
}

#[test]
fn environment_references_resolve_for_alerting_args() {
    let source = "general_settings:\n  alerting_args:\n    daily_report_frequency: os.environ/FREQUENCY\n    spend_anomaly_multiplier: os.environ/MULTIPLIER";
    let args = settings(source, &[("FREQUENCY", "90"), ("MULTIPLIER", "4.5")]).alerting_args;
    assert_eq!(args.daily_report_frequency, 90);
    assert_eq!(args.spend_anomaly_multiplier, decimal("4.5"));
}

#[test]
fn a_missing_alerting_args_variable_is_an_error_naming_the_field() {
    let source = "general_settings:\n  alerting_args:\n    report_check_interval: os.environ/CHECK";
    let error = load_error(source, &[]);
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "CHECK"
                && path == "general_settings.alerting_args.report_check_interval"
    ));
}

#[rstest]
#[case::zero_spend_threshold("daily_spend_per_user_threshold", "0")]
#[case::negative_spend_threshold("daily_spend_per_user_threshold", "-1")]
#[case::zero_monthly_threshold("monthly_spend_per_user_threshold", "0")]
#[case::zero_anomaly_multiplier("spend_anomaly_multiplier", "0")]
#[case::zero_min_spend("spend_anomaly_min_spend", "0")]
#[case::zero_baseline_days("spend_anomaly_baseline_days", "0")]
#[case::short_check_interval("user_spend_check_interval", "59")]
fn an_out_of_range_alerting_arg_is_an_error_naming_the_field(
    #[case] field: &str,
    #[case] value: &str,
) {
    let source = format!("general_settings:\n  alerting_args:\n    {field}: {value}");
    let error = load_error(&source, &[]);
    assert_eq!(error.category(), "invalid_scalar");
    assert!(
        error
            .to_string()
            .contains(&format!("general_settings.alerting_args.{field}")),
        "{error}"
    );
}

#[rstest]
#[case::smallest_positive_threshold("daily_spend_per_user_threshold", "0.01", |args: &SlackAlertingArgs| args.daily_user_spend_threshold == Some(decimal("0.01")))]
#[case::smallest_anomaly_multiplier("spend_anomaly_multiplier", "0.5", |args: &SlackAlertingArgs| args.spend_anomaly_multiplier == decimal("0.5"))]
#[case::smallest_anomaly_min_spend("spend_anomaly_min_spend", "0.01", |args: &SlackAlertingArgs| args.spend_anomaly_min_spend == decimal("0.01"))]
#[case::one_baseline_day("spend_anomaly_baseline_days", "1", |args: &SlackAlertingArgs| args.spend_anomaly_baseline_days == 1)]
#[case::shortest_check_interval("user_spend_check_interval", "60", |args: &SlackAlertingArgs| args.user_spend_check_interval == 60)]
fn the_smallest_allowed_alerting_arg_loads(
    #[case] field: &str,
    #[case] value: &str,
    #[case] applies: fn(&SlackAlertingArgs) -> bool,
) {
    let source = format!("general_settings:\n  alerting_args:\n    {field}: {value}");
    assert!(applies(&settings(&source, &[]).alerting_args));
}

#[test]
fn alert_types_parse_in_config_order() {
    let source = "general_settings:\n  alert_types:\n    - daily_reports\n    - llm_exceptions\n    - user_spend_anomalies";
    let loaded = settings(source, &[]);
    assert_eq!(
        loaded.alert_types,
        vec![
            AlertType::DailyReports,
            AlertType::LlmExceptions,
            AlertType::UserSpendAnomalies
        ]
    );
}

#[test]
fn an_explicit_empty_alert_types_list_disables_every_alert() {
    let loaded = settings("general_settings:\n  alert_types: []", &[]);
    assert!(loaded.alert_types.is_empty());
}

#[test]
fn null_alert_types_take_the_defaults() {
    let loaded = settings("general_settings:\n  alert_types: null", &[]);
    assert_eq!(loaded.alert_types, DEFAULT_ALERT_TYPES.to_vec());
}

#[test]
fn an_unknown_alert_type_name_is_rejected() {
    let error = load_error("general_settings:\n  alert_types:\n    - not_an_alert", &[]);
    assert!(matches!(error, ConfigError::Yaml { .. }));
}

#[test]
fn alerting_destinations_keep_their_order_and_duplicates() {
    let loaded = settings(
        "general_settings:\n  alerting:\n    - slack\n    - webhook\n    - slack",
        &[],
    );
    assert_eq!(loaded.alerting, vec!["slack", "webhook", "slack"]);
}

#[test]
fn alerting_threshold_defaults_to_300_seconds() {
    let loaded = settings("general_settings: {}", &[]);
    assert_eq!(
        loaded.alerting_threshold_seconds.to_bits(),
        300.0_f64.to_bits()
    );
}

#[test]
fn an_enabled_alerting_defaults_to_600_seconds() {
    let loaded = settings("general_settings:\n  alerting:\n    - slack", &[]);
    assert_eq!(
        loaded.alerting_threshold_seconds.to_bits(),
        600.0_f64.to_bits()
    );
}

#[test]
fn a_null_alerting_threshold_keeps_300_seconds() {
    let loaded = settings(
        "general_settings:\n  alerting:\n    - slack\n  alerting_threshold: null",
        &[],
    );
    assert_eq!(
        loaded.alerting_threshold_seconds.to_bits(),
        300.0_f64.to_bits()
    );
}

#[test]
fn a_threshold_without_alerting_stays_at_300_seconds() {
    let loaded = settings("general_settings:\n  alerting_threshold: 900", &[]);
    assert_eq!(
        loaded.alerting_threshold_seconds.to_bits(),
        300.0_f64.to_bits()
    );
}

#[rstest]
#[case::fractional("600.5", 600.5_f64)]
#[case::zero("0", 0.0_f64)]
fn an_explicit_alerting_threshold_is_used_as_given(#[case] text: &str, #[case] expected: f64) {
    let source =
        format!("general_settings:\n  alerting:\n    - slack\n  alerting_threshold: {text}");
    let loaded = settings(&source, &[]);
    assert_eq!(
        loaded.alerting_threshold_seconds.to_bits(),
        expected.to_bits()
    );
}

#[test]
fn webhook_urls_resolve_environment_references() {
    let source = "general_settings:\n  alert_to_webhook_url:\n    budget_alerts:\n      - https://hooks.example/plain\n      - os.environ/SECOND_HOOK\n    spend_reports: os.environ/FIRST_HOOK";
    let loaded = settings(
        source,
        &[("FIRST_HOOK", "https://a"), ("SECOND_HOOK", "https://b")],
    );
    let budgets = &loaded.alert_to_webhook_url[&AlertType::BudgetAlerts];
    assert!(matches!(budgets, WebhookUrl::Multiple(_)));
    assert_eq!(
        exposed(budgets),
        vec!["https://hooks.example/plain", "https://b"]
    );
    let reports = &loaded.alert_to_webhook_url[&AlertType::SpendReports];
    assert!(matches!(reports, WebhookUrl::Single(_)));
    assert_eq!(exposed(reports), vec!["https://a"]);
}

#[test]
fn a_plain_webhook_url_is_kept_unchanged() {
    let source =
        "general_settings:\n  alert_to_webhook_url:\n    budget_alerts: https://hooks.example/x";
    let loaded = settings(source, &[]);
    let webhook = &loaded.alert_to_webhook_url[&AlertType::BudgetAlerts];
    assert!(matches!(webhook, WebhookUrl::Single(_)));
    assert_eq!(exposed(webhook), vec!["https://hooks.example/x"]);
}

#[test]
fn an_empty_webhook_url_is_used_as_given() {
    let source = "general_settings:\n  alert_to_webhook_url:\n    budget_alerts: \"\"";
    let loaded = settings(source, &[]);
    let webhook = &loaded.alert_to_webhook_url[&AlertType::BudgetAlerts];
    assert!(matches!(webhook, WebhookUrl::Single(_)));
    assert_eq!(exposed(webhook), vec![String::new()]);
}

#[test]
fn settings_debug_hides_the_webhook_urls() {
    let source = "general_settings:\n  alert_to_webhook_url:\n    budget_alerts: https://hooks.example/SECRET";
    let loaded = settings(source, &[]);
    let debug = format!("{loaded:?}");
    assert!(!debug.contains("SECRET"), "{debug}");
    assert!(debug.contains("REDACTED"), "{debug}");
}

#[test]
fn a_missing_webhook_variable_is_an_error_naming_the_variable_and_field() {
    let source = "general_settings:\n  alert_to_webhook_url:\n    budget_alerts:\n      - os.environ/MISSING_HOOK";
    let error = load_error(source, &[]);
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "MISSING_HOOK"
                && path == "general_settings.alert_to_webhook_url.budget_alerts[0]"
    ));
}

#[test]
fn a_webhook_reference_must_begin_with_the_marker() {
    let source = "general_settings:\n  alert_to_webhook_url:\n    budget_alerts: https://hooks.example/os.environ/HOOK";
    let error = load_error(source, &[]);
    assert!(matches!(
        &error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "HOOK" && path == "general_settings.alert_to_webhook_url.budget_alerts"
    ));
    assert!(!error.to_string().contains("hooks.example"), "{error}");
}

#[test]
fn an_unknown_alert_type_as_a_webhook_key_is_rejected() {
    let source =
        "general_settings:\n  alert_to_webhook_url:\n    not_an_alert: https://hooks.example/x";
    assert!(matches!(load_error(source, &[]), ConfigError::Yaml { .. }));
}

#[test]
fn digest_settings_default_when_the_block_is_empty() {
    let loaded = settings(
        "general_settings:\n  alert_type_config:\n    spend_reports: {}",
        &[],
    );
    let config = loaded.alert_type_config[&AlertType::SpendReports];
    assert!(!config.digest);
    assert_eq!(config.digest_interval, 86_400);
}

#[test]
fn digest_settings_parse_from_config() {
    let source = "general_settings:\n  alert_type_config:\n    spend_reports:\n      digest: true\n      digest_interval: 60";
    let loaded = settings(source, &[]);
    let config = loaded.alert_type_config[&AlertType::SpendReports];
    assert!(config.digest);
    assert_eq!(config.digest_interval, 60);
}

#[test]
fn an_unknown_alert_type_in_alert_type_config_is_rejected() {
    let source = "general_settings:\n  alert_type_config:\n    not_an_alert:\n      digest: true";
    assert!(matches!(load_error(source, &[]), ConfigError::Yaml { .. }));
}

#[test]
fn an_unknown_alerting_args_key_is_reported() {
    let source = "general_settings:\n  alerting_args:\n    not_an_arg: 1";
    let error = load_error(source, &[]);
    assert_eq!(error.category(), "unsupported_key");
    assert!(error.to_string().contains("not_an_arg"), "{error}");
}

#[test]
fn log_to_console_resolves_an_environment_reference() {
    let loaded = settings(
        "general_settings:\n  alerting_args:\n    log_to_console: os.environ/LOG_ALERTS",
        &[("LOG_ALERTS", "true")],
    );
    assert!(loaded.alerting_args.log_to_console);
}

#[test]
fn a_missing_log_to_console_variable_is_an_error_naming_the_field() {
    let source = "general_settings:\n  alerting_args:\n    log_to_console: os.environ/LOG_ALERTS";
    let error = load_error(source, &[]);
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "LOG_ALERTS"
                && path == "general_settings.alerting_args.log_to_console"
    ));
}

#[test]
fn digest_settings_resolve_environment_references() {
    let source = "general_settings:\n  alert_type_config:\n    spend_reports:\n      digest: os.environ/DIGEST\n      digest_interval: os.environ/DIGEST_SECONDS";
    let loaded = settings(source, &[("DIGEST", "yes"), ("DIGEST_SECONDS", "120")]);
    let config = loaded.alert_type_config[&AlertType::SpendReports];
    assert!(config.digest);
    assert_eq!(config.digest_interval, 120);
}

#[test]
fn a_missing_digest_variable_is_an_error_naming_the_field() {
    let source = "general_settings:\n  alert_type_config:\n    spend_reports:\n      digest: os.environ/DIGEST";
    let error = load_error(source, &[]);
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "DIGEST"
                && path == "general_settings.alert_type_config.spend_reports.digest"
    ));
}

#[test]
fn a_missing_digest_interval_variable_is_an_error_naming_the_field() {
    let source = "general_settings:\n  alert_type_config:\n    spend_reports:\n      digest_interval: os.environ/DIGEST_SECONDS";
    let error = load_error(source, &[]);
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "DIGEST_SECONDS"
                && path == "general_settings.alert_type_config.spend_reports.digest_interval"
    ));
}

#[test]
fn an_unknown_alert_type_config_key_is_reported() {
    let source = "general_settings:\n  alert_type_config:\n    spend_reports:\n      note: x";
    let error = load_error(source, &[]);
    assert_eq!(error.category(), "unsupported_key");
    assert!(error.to_string().contains("note"), "{error}");
}

#[test]
fn a_negative_digest_interval_is_rejected() {
    let source =
        "general_settings:\n  alert_type_config:\n    spend_reports:\n      digest_interval: -1";
    let error = load_error(source, &[]);
    assert_eq!(error.category(), "invalid_integer");
}
