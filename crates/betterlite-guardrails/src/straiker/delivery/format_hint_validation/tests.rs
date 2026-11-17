#![cfg(test)]

use rstest::rstest;

use crate::straiker::delivery::fixtures::{raw, settings, v3_raw};
use crate::straiker::delivery::settings::{FormatHint, SettingsError, StraikerSettings};

#[rstest]
#[case::unset(None, None)]
#[case::anthropic(Some("anthropic.messages"), Some(FormatHint::AnthropicMessages))]
#[case::openai(Some("openai.chat"), Some(FormatHint::OpenAiChat))]
fn a_known_hint_resolves(#[case] configured: Option<&str>, #[case] expected: Option<FormatHint>) {
    let mut args = raw();
    args.format_hint = configured.map(str::to_owned);
    assert_eq!(settings(&args).format_hint, expected);
}

#[rstest]
#[case::bare_family("anthropic")]
#[case::empty("")]
#[case::swapped("openai.messages")]
fn an_unknown_hint_is_rejected(#[case] value: &str) {
    let mut args = raw();
    args.format_hint = Some(value.to_owned());
    let error = StraikerSettings::resolve(&args).expect_err("the hint is unknown");
    assert_eq!(
        error.to_string(),
        format!("format_hint must be 'anthropic.messages' or 'openai.chat'; got {value:?}")
    );
    assert!(matches!(error, SettingsError::FormatHint { .. }));
}

#[test]
fn a_hint_is_accepted_with_a_v3_key() {
    let mut args = v3_raw();
    args.format_hint = Some("openai.chat".to_owned());
    assert_eq!(settings(&args).format_hint, Some(FormatHint::OpenAiChat));
}

#[test]
fn the_hint_is_reported_before_the_backoff_values() {
    let mut args = raw();
    args.format_hint = Some("anthropic".to_owned());
    args.initial_backoff = Some(f64::INFINITY);
    let error = StraikerSettings::resolve(&args).expect_err("the hint is unknown");
    assert!(matches!(error, SettingsError::FormatHint { .. }));
}
