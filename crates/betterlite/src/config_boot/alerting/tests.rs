#![cfg(test)]

use std::io::Error;
use std::sync::Mutex;

use betterlite_config::sections::settings::JsonValue;

use super::{AlertingPort, AlertingUpdate, load_alerting_settings};
use crate::config_boot::error::ConfigBootError;
use crate::config_boot::test_support::{capture_logs, config_from_yaml};

#[derive(Default)]
struct RecordingAlerting {
    updates: Mutex<Vec<String>>,
    loggers: Mutex<Vec<String>>,
    fail_logger_for: Option<&'static str>,
}

impl RecordingAlerting {
    fn updates(&self) -> Vec<String> {
        self.updates.lock().unwrap().clone()
    }

    fn loggers(&self) -> Vec<String> {
        self.loggers.lock().unwrap().clone()
    }

    fn failing_logger_for(name: &'static str) -> Self {
        Self {
            updates: Mutex::new(Vec::new()),
            loggers: Mutex::new(Vec::new()),
            fail_logger_for: Some(name),
        }
    }
}

impl AlertingPort for RecordingAlerting {
    fn update_alerting(&self, update: &AlertingUpdate<'_>) -> Result<(), ConfigBootError> {
        let call = format!(
            "alerting:{}:threshold:{}:types:{}:webhook:{}:args:{}:config:{}",
            update.alerting.join(","),
            threshold_text(update.alerting_threshold),
            update.alert_types.is_some(),
            update.alert_to_webhook_url.is_some(),
            update.alerting_args.is_some(),
            update.alert_type_config.is_some()
        );
        self.updates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(call);
        Ok(())
    }

    fn add_alerting_logger(
        &self,
        name: &str,
        alerting_args: Option<&JsonValue>,
    ) -> Result<(), ConfigBootError> {
        self.loggers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("{name}:{}", alerting_args.is_some()));
        if self.fail_logger_for == Some(name) {
            return Err(ConfigBootError::Step {
                step: "test alerting logger",
                source: Box::new(Error::other("the test logger failed")),
            });
        }
        Ok(())
    }
}

fn threshold_text(threshold: Option<f64>) -> String {
    threshold.map_or_else(|| "None".to_owned(), |value| format!("Some({value})"))
}

fn settings_of(yaml: &str) -> JsonValue {
    let config = config_from_yaml(yaml);
    config.get("general_settings").cloned().unwrap_or_default()
}

#[test]
fn missing_general_settings_is_skipped() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(&JsonValue::Null, &ports).unwrap();
    assert!(ports.updates().is_empty());
    assert!(ports.loggers().is_empty());
}

#[test]
fn absent_alerting_is_skipped() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(
        &settings_of("general_settings:\n  master_key: sk\n"),
        &ports,
    )
    .unwrap();
    assert!(ports.updates().is_empty());
}

#[test]
fn null_alerting_is_skipped() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(&settings_of("general_settings:\n  alerting:\n"), &ports).unwrap();
    assert!(ports.updates().is_empty());
}

#[test]
fn a_non_mapping_general_settings_is_refused() {
    let ports = RecordingAlerting::default();
    let settings = JsonValue::from("hello");
    let error = load_alerting_settings(&settings, &ports).unwrap_err();
    assert_eq!(
        error.to_string(),
        "config field 'general_settings' must be a mapping"
    );
}

#[test]
fn an_empty_alerting_list_still_reaches_update_alerting() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(&settings_of("general_settings:\n  alerting: []\n"), &ports).unwrap();
    assert_eq!(
        ports.updates(),
        vec!["alerting::threshold:Some(600):types:false:webhook:false:args:false:config:false"]
    );
}

#[test]
fn slack_reaches_update_alerting_but_not_the_logger_loop() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(
        &settings_of("general_settings:\n  alerting:\n    - slack\n"),
        &ports,
    )
    .unwrap();
    assert_eq!(
        ports.updates(),
        vec![
            "alerting:slack:threshold:Some(600):types:false:webhook:false:args:false:config:false"
        ]
    );
    assert!(ports.loggers().is_empty());
}

#[test]
fn known_callbacks_are_registered_with_the_alerting_args() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(
        &settings_of(
            "general_settings:\n  alerting:\n    - pagerduty\n  alerting_args:\n    api_key: sk\n",
        ),
        &ports,
    )
    .unwrap();
    assert_eq!(ports.loggers(), vec!["pagerduty:true".to_owned()]);
}

#[test]
fn unknown_callbacks_are_skipped() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(
        &settings_of("general_settings:\n  alerting:\n    - not_a_real_integration\n"),
        &ports,
    )
    .unwrap();
    assert!(ports.loggers().is_empty());
    assert_eq!(ports.updates().len(), 1);
}

#[test]
fn the_threshold_defaults_to_600() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(
        &settings_of("general_settings:\n  alerting:\n    - slack\n"),
        &ports,
    )
    .unwrap();
    assert!(
        ports
            .updates()
            .first()
            .unwrap()
            .contains("threshold:Some(600)")
    );
}

#[test]
fn an_explicit_null_threshold_passes_none() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(
        &settings_of("general_settings:\n  alerting:\n    - slack\n  alerting_threshold: null\n"),
        &ports,
    )
    .unwrap();
    assert!(ports.updates().first().unwrap().contains("threshold:None"));
}

#[test]
fn a_configured_threshold_is_passed() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(
        &settings_of("general_settings:\n  alerting:\n    - slack\n  alerting_threshold: 30\n"),
        &ports,
    )
    .unwrap();
    assert!(
        ports
            .updates()
            .first()
            .unwrap()
            .contains("threshold:Some(30)")
    );
}

#[test]
fn the_raw_alerting_settings_are_forwarded() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(
        &settings_of(
            "general_settings:\n  alerting:\n    - slack\n  alert_types:\n    - budget_alerts\n  alert_to_webhook_url: https://example.test/hook\n  alerting_args:\n    a: b\n  alert_type_config:\n    budget_alerts:\n      a: b\n",
        ),
        &ports,
    )
    .unwrap();
    assert!(
        ports
            .updates()
            .first()
            .unwrap()
            .contains("types:true:webhook:true:args:true:config:true")
    );
}

#[test]
fn absent_raw_alerting_settings_are_none() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(
        &settings_of("general_settings:\n  alerting:\n    - slack\n"),
        &ports,
    )
    .unwrap();
    assert!(
        ports
            .updates()
            .first()
            .unwrap()
            .contains("types:false:webhook:false:args:false:config:false")
    );
}

#[test]
fn explicit_null_raw_alerting_settings_are_none() {
    let ports = RecordingAlerting::default();
    load_alerting_settings(
        &settings_of(
            "general_settings:\n  alerting:\n    - slack\n  alert_types:\n  alert_to_webhook_url:\n  alerting_args:\n  alert_type_config:\n",
        ),
        &ports,
    )
    .unwrap();
    assert!(
        ports
            .updates()
            .first()
            .unwrap()
            .contains("types:false:webhook:false:args:false:config:false")
    );
}

#[test]
fn alerting_must_be_a_list() {
    let ports = RecordingAlerting::default();
    let error = load_alerting_settings(
        &settings_of("general_settings:\n  alerting: slack\n"),
        &ports,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("reading general_settings.alerting failed")
    );
}

#[test]
fn a_non_numeric_threshold_is_refused() {
    let ports = RecordingAlerting::default();
    let error = load_alerting_settings(
        &settings_of("general_settings:\n  alerting:\n    - slack\n  alerting_threshold: soon\n"),
        &ports,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("reading general_settings.alerting_threshold failed")
    );
}

#[test]
fn a_failing_logger_is_logged_and_boot_continues() {
    let (logs, _guard) = capture_logs();
    let ports = RecordingAlerting::failing_logger_for("datadog");
    load_alerting_settings(
        &settings_of("general_settings:\n  alerting:\n    - datadog\n    - pagerduty\n"),
        &ports,
    )
    .unwrap();
    assert_eq!(
        ports.loggers(),
        vec!["datadog:false".to_owned(), "pagerduty:false".to_owned()]
    );
    assert!(logs.contents().contains(
        "[Non-Blocking Error] Error initializing custom logger: test alerting logger failed"
    ));
}
