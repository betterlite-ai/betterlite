use std::time::Duration;

use crate::datadog_llm_observability::error::DatadogLlmObsError;
use crate::datadog_llm_observability::tests::support;

#[test]
fn datadog_llm_obs_config_resolution_behavior() {
    let environment = support::environment(&[]);
    let mut init = support::agent_init();
    init.params = Some(support::params(Some(true)));
    init.turn_off_message_logging = Some(false);
    let params_on = support::resolve(&init, &environment).expect("the settings resolve");
    assert!(params_on.options.turn_off_message_logging);
    init.params = Some(support::params(Some(false)));
    init.turn_off_message_logging = Some(true);
    let params_off = support::resolve(&init, &environment).expect("the settings resolve");
    assert!(!params_off.options.turn_off_message_logging);
    init.params = None;
    init.turn_off_message_logging = Some(true);
    let constructor_on = support::resolve(&init, &environment).expect("the settings resolve");
    assert!(constructor_on.options.turn_off_message_logging);
    init.message_logging = Some(false);
    let message_logging_off = support::resolve(&init, &environment).expect("the settings resolve");
    assert!(!message_logging_off.options.message_logging);
    assert!(message_logging_off.options.turn_off_message_logging);
}

#[test]
fn datadog_llm_obs_message_logging_defaults_on() {
    let environment = support::environment(&[]);
    let settings =
        support::resolve(&support::agent_init(), &environment).expect("the settings resolve");
    assert!(settings.options.message_logging);
    assert!(!settings.options.turn_off_message_logging);
}

#[test]
fn datadog_llm_obs_batch_settings_follow_the_environment() {
    let environment = support::environment(&[
        ("DEFAULT_BATCH_SIZE", "3"),
        ("DEFAULT_FLUSH_INTERVAL_SECONDS", "7"),
    ]);
    let settings =
        support::resolve(&support::agent_init(), &environment).expect("the settings resolve");
    assert_eq!(settings.batch.batch_size, 3);
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(7));
}

#[test]
fn datadog_llm_obs_batch_settings_default_when_unset() {
    let environment = support::environment(&[
        ("DEFAULT_BATCH_SIZE", ""),
        ("DEFAULT_FLUSH_INTERVAL_SECONDS", ""),
    ]);
    let settings =
        support::resolve(&support::agent_init(), &environment).expect("the settings resolve");
    assert_eq!(settings.batch.batch_size, 512);
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(5));
}

#[test]
fn datadog_llm_obs_constructor_batch_settings_override_the_environment() {
    let environment = support::environment(&[
        ("DEFAULT_BATCH_SIZE", "3"),
        ("DEFAULT_FLUSH_INTERVAL_SECONDS", "7"),
    ]);
    let mut init = support::agent_init();
    init.batch_size = Some(2);
    init.flush_interval_seconds = Some(0.5_f64);
    let settings = support::resolve(&init, &environment).expect("the settings resolve");
    assert_eq!(settings.batch.batch_size, 2);
    assert_eq!(settings.batch.flush_interval, Duration::from_millis(500));
}

#[test]
fn datadog_llm_obs_an_invalid_batch_size_names_the_variable() {
    let environment = support::environment(&[("DEFAULT_BATCH_SIZE", "many")]);
    let error =
        support::resolve(&support::agent_init(), &environment).expect_err("the size is refused");
    assert!(matches!(error, DatadogLlmObsError::Environment(_)));
    assert!(error.to_string().contains("DEFAULT_BATCH_SIZE"));
}

#[test]
fn datadog_llm_obs_a_negative_flush_interval_is_refused() {
    let environment = support::environment(&[]);
    let mut init = support::agent_init();
    init.flush_interval_seconds = Some(-1.0_f64);
    let error = support::resolve(&init, &environment).expect_err("the interval is refused");
    assert!(matches!(error, DatadogLlmObsError::FlushInterval { .. }));
}
