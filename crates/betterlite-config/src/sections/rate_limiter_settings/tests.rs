#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

const WINDOW_SIZE: &str = "LITELLM_RATE_LIMIT_WINDOW_SIZE";
const TPM_RESERVATION: &str = "LITELLM_TPM_TOKEN_RESERVATION_ENABLED";

fn lookup(environment: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings(source: &str, environment: &[(&str, &str)]) -> Settings {
    ConfigLoader {
        lookup: lookup(environment),
    }
    .load(source)
    .expect("the config should load")
    .sections
    .rate_limiter_settings
}

fn load_error(source: &str, environment: &[(&str, &str)]) -> ConfigError {
    ConfigLoader {
        lookup: lookup(environment),
    }
    .load(source)
    .expect_err("the environment should be rejected")
}

fn flag_source(rendered: &str) -> String {
    format!("general_settings:\n  fail_closed_rate_limit_enforcement: {rendered}\n")
}

fn flag(rendered: &str, environment: &[(&str, &str)]) -> bool {
    settings(&flag_source(rendered), environment).fail_closed
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut output) = self.0.lock() else {
            return Err(io::Error::other("poisoned capture"));
        };
        output.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture<F>(log: F) -> String
where
    F: FnOnce(),
{
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&output);
    let subscriber = fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(Level::TRACE)
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    with_default(subscriber, log);
    let bytes = output.lock().expect("the capture is not poisoned").clone();
    String::from_utf8(bytes).expect("the capture is UTF-8")
}

#[test]
fn defaults_match_the_pinned_values() {
    let settings = settings("{}", &[]);
    assert!(!settings.fail_closed);
    assert_eq!(settings.window_size, 60);
    assert!(settings.tpm_reservation_enabled);
}

#[rstest]
#[case::absent("{}".to_owned(), false)]
#[case::null(flag_source("null"), false)]
#[case::empty_general_settings("general_settings: {}\n".to_owned(), false)]
#[case::enabled(flag_source("true"), true)]
#[case::disabled(flag_source("false"), false)]
#[case::enabled_string(flag_source("\"true\""), true)]
#[case::upper_case_string(flag_source("\"TRUE\""), true)]
#[case::yes_string(flag_source("\"yes\""), true)]
#[case::one_string(flag_source("\"1\""), true)]
#[case::on_string(flag_source("\"on\""), true)]
#[case::false_string(flag_source("\"false\""), false)]
#[case::zero_string(flag_source("\"0\""), false)]
#[case::no_string(flag_source("\"no\""), false)]
#[case::off_string(flag_source("\"off\""), false)]
fn the_fail_closed_flag_reads_bools_and_strings(#[case] source: String, #[case] expected: bool) {
    assert_eq!(settings(&source, &[]).fail_closed, expected);
}

#[rstest]
#[case::integer(flag_source("1"), "1")]
#[case::float(flag_source("1.5"), "1.5")]
#[case::list(flag_source("[true]"), "a list or a map")]
#[case::map(flag_source("{a: 1}"), "a list or a map")]
#[case::unreadable_string(flag_source("\"maybe\""), "maybe")]
fn a_value_outside_the_declared_type_warns_and_disables(
    #[case] source: String,
    #[case] rendered: &str,
) {
    let text = capture(|| {
        assert!(!settings(&source, &[]).fail_closed);
    });
    assert!(text.contains("WARN"));
    assert!(text.contains(&format!(
        "general_settings.fail_closed_rate_limit_enforcement={rendered} is not a boolean, treating it as disabled"
    )));
}

#[test]
fn a_bool_flag_logs_no_warning() {
    let text = capture(|| {
        assert!(settings(&flag_source("true"), &[]).fail_closed);
        assert!(!settings(&flag_source("false"), &[]).fail_closed);
    });
    assert!(!text.contains("fail_closed_rate_limit_enforcement"));
}

#[rstest]
#[case::resolved_environment(&[("FAIL_CLOSED_FLAG", "true")], true)]
#[case::resolved_false_environment(&[("FAIL_CLOSED_FLAG", "off")], false)]
#[case::unset_environment(&[], false)]
#[case::empty_environment(&[("FAIL_CLOSED_FLAG", "")], false)]
fn an_environment_reference_is_read_through_the_shared_bool_parser(
    #[case] environment: &[(&str, &str)],
    #[case] expected: bool,
) {
    assert_eq!(
        flag("\"os.environ/FAIL_CLOSED_FLAG\"", environment),
        expected
    );
}

#[rstest]
#[case::unset(60, &[])]
#[case::configured(120, &[(WINDOW_SIZE, "120")])]
#[case::empty_counts_as_unset(60, &[(WINDOW_SIZE, "")])]
#[case::surrounding_spaces(90, &[(WINDOW_SIZE, " 90 ")])]
fn the_window_size_reads_the_environment(
    #[case] expected: i64,
    #[case] environment: &[(&str, &str)],
) {
    assert_eq!(settings("{}", environment).window_size, expected);
}

#[rstest]
#[case::zero("0")]
#[case::negative("-5")]
#[case::fractional("60.5")]
#[case::text("one minute")]
#[case::exponent("1e3")]
fn a_non_positive_or_non_integer_window_size_is_a_load_error(#[case] value: &str) {
    let error = load_error("{}", &[(WINDOW_SIZE, value)]);
    assert!(
        error.to_string().contains(WINDOW_SIZE),
        "the error names the variable: {error}"
    );
}

#[rstest]
#[case::unset(true, &[])]
#[case::enabled(true, &[(TPM_RESERVATION, "true")])]
#[case::upper_case(true, &[(TPM_RESERVATION, "TRUE")])]
#[case::disabled(false, &[(TPM_RESERVATION, "false")])]
#[case::one_disables(false, &[(TPM_RESERVATION, "1")])]
#[case::yes_disables(false, &[(TPM_RESERVATION, "yes")])]
#[case::on_disables(false, &[(TPM_RESERVATION, "on")])]
#[case::empty_disables(false, &[(TPM_RESERVATION, "")])]
fn the_reservation_flag_is_true_only_for_the_text_true(
    #[case] expected: bool,
    #[case] environment: &[(&str, &str)],
) {
    assert_eq!(
        settings("{}", environment).tpm_reservation_enabled,
        expected
    );
}

#[test]
fn the_three_settings_load_together() {
    let settings = settings(
        &flag_source("true"),
        &[(WINDOW_SIZE, "30"), (TPM_RESERVATION, "false")],
    );
    assert!(settings.fail_closed);
    assert_eq!(settings.window_size, 30);
    assert!(!settings.tpm_reservation_enabled);
}
