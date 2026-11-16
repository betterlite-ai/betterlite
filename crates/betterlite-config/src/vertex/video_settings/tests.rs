#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use super::video_settings::{Settings, VIDEO_DURATION_ENV};
use crate::loader::{ConfigError, ConfigLoader};

fn load(environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load("")?;
    Ok(config.sections.vertex_video_settings)
}

#[rstest]
#[case::absent(&[], 8)]
#[case::an_override_replaces_the_default(&[(VIDEO_DURATION_ENV, "11")], 11)]
#[case::an_explicit_zero_is_used(&[(VIDEO_DURATION_ENV, "0")], 0)]
#[case::empty_text_counts_as_unset(&[(VIDEO_DURATION_ENV, "")], 8)]
#[case::blank_text_counts_as_unset(&[(VIDEO_DURATION_ENV, "  ")], 8)]
#[case::surrounding_spaces_are_trimmed(&[(VIDEO_DURATION_ENV, " 11 ")], 11)]
fn loads_default_google_video_duration(
    #[case] environment: &[(&str, &str)],
    #[case] expected: i64,
) {
    let settings = load(environment).expect("the duration loads");
    assert_eq!(settings.default_video_duration_seconds, expected);
}

#[rstest]
#[case::words("eight")]
#[case::a_fraction("8.0")]
#[case::separators("1_0")]
#[case::beyond_the_integer_range("99999999999999999999")]
fn loads_default_google_video_duration_rejects_an_invalid_integer(#[case] text: &str) {
    let error = load(&[(VIDEO_DURATION_ENV, text)]).unwrap_err();
    assert!(matches!(
        &error,
        ConfigError::Environment { source: EnvError::Number { name } } if name == VIDEO_DURATION_ENV
    ));
    assert!(!error.to_string().contains(text));
}
