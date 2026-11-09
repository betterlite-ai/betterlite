use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_config::loader::EnvironmentLookup;
use rstest::rstest;

use super::fixtures::{capture_warnings, content_policy, environment};
use crate::newrelic_extraction::content::{ContentPolicy, RECORD_CONTENT_ENV};

struct ToggleEnv(AtomicBool);

impl EnvironmentLookup for ToggleEnv {
    fn lookup(&self, _name: &str) -> Option<String> {
        let value = if self.0.load(Ordering::Relaxed) {
            "true"
        } else {
            "false"
        };
        Some(value.to_owned())
    }
}

fn env_of(raw: Option<&str>) -> BTreeMap<String, String> {
    raw.map_or_else(
        || environment(&[]),
        |value| environment(&[(RECORD_CONTENT_ENV, value)]),
    )
}

#[rstest]
#[case::true_word("true", true)]
#[case::one("1", true)]
#[case::yes("yes", true)]
#[case::on("on", true)]
#[case::upper_case("TRUE", true)]
#[case::padded(" yes ", true)]
#[case::false_word("false", false)]
#[case::zero("0", false)]
#[case::no("no", false)]
#[case::off("off", false)]
#[case::mixed_case("oFf", false)]
fn newrelic_extraction_tests_behavior_record_content(#[case] raw: &str, #[case] expected: bool) {
    let env = environment(&[(RECORD_CONTENT_ENV, raw)]);
    let policy = content_policy(&env);
    assert_eq!(policy.record_content(), expected);
}

#[rstest]
#[case::unset(None)]
#[case::empty(Some(""))]
#[case::spaces(Some("   "))]
#[case::unrecognized(Some("maybe"))]
fn newrelic_extraction_tests_behavior_record_content_defaults(#[case] raw: Option<&str>) {
    let env = env_of(raw);
    let policy = content_policy(&env);
    assert!(policy.record_content());
}

#[test]
fn newrelic_extraction_tests_behavior_unrecognized_boolean_warns() {
    let (buffer, guard) = capture_warnings();
    let env = environment(&[(RECORD_CONTENT_ENV, "maybe")]);
    let policy = content_policy(&env);
    assert!(policy.record_content());
    drop(guard);
    let log = buffer.text();
    assert_eq!(log.matches(RECORD_CONTENT_ENV).count(), 1, "{log}");
    assert!(log.contains("not a recognised boolean"), "{log}");
}

#[test]
fn newrelic_extraction_tests_behavior_recognized_boolean_does_not_warn() {
    let (buffer, guard) = capture_warnings();
    let env = environment(&[(RECORD_CONTENT_ENV, "true")]);
    let policy = content_policy(&env);
    assert!(policy.record_content());
    drop(guard);
    let log = buffer.text();
    assert!(log.is_empty(), "{log}");
}

#[test]
fn newrelic_extraction_tests_behavior_record_content_ignores_other_variables() {
    let env = environment(&[("NEW_RELIC_AI_MONITORING_RECORD_CONTENT", "false")]);
    let policy = content_policy(&env);
    assert!(policy.record_content());
}

#[rstest]
#[case::environment_on(Some("true"))]
#[case::environment_off(Some("false"))]
#[case::environment_unset(None)]
fn newrelic_extraction_tests_behavior_turn_off_disables_content(#[case] raw: Option<&str>) {
    let env = env_of(raw);
    let mut policy = content_policy(&env);
    policy.turn_off_message_logging = true;
    assert!(!policy.record_content());
}

#[test]
fn newrelic_extraction_tests_behavior_redaction_does_not_change_record_content() {
    let env = environment(&[]);
    let mut policy = content_policy(&env);
    policy.redact_message_logging = true;
    assert!(policy.record_content());
}

#[test]
fn newrelic_extraction_tests_behavior_record_content_reads_the_environment_each_call() {
    let env = ToggleEnv(AtomicBool::new(true));
    let policy = ContentPolicy {
        turn_off_message_logging: false,
        redact_message_logging: false,
        environment: &env,
    };
    assert!(policy.record_content());
    env.0.store(false, Ordering::Relaxed);
    assert!(!policy.record_content());
    env.0.store(true, Ordering::Relaxed);
    assert!(policy.record_content());
}
