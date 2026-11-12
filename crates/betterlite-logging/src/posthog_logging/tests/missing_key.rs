use crate::posthog_logging::settings::{PostHogSettingsError, resolve};
use crate::posthog_logging::tests::support::{API_KEY, environment};

#[test]
fn posthog_missing_key_names_the_variable() {
    let error = resolve(&environment(&[])).expect_err("the key is absent");
    assert_eq!(
        error.to_string(),
        "POSTHOG_API_KEY is not set, set 'POSTHOG_API_KEY=<>'"
    );
    assert!(matches!(error, PostHogSettingsError::MissingApiKey));
}

#[test]
fn posthog_missing_key_accepts_a_set_key() {
    let settings =
        resolve(&environment(&[("POSTHOG_API_KEY", API_KEY)])).expect("the settings resolve");
    assert_eq!(settings.capture_url, "https://us.i.posthog.com/batch/");
}
