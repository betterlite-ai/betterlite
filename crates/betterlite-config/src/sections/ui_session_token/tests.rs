#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

const DURATION_ENV: &str = "LITELLM_UI_SESSION_DURATION";

fn load(environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load("")?.sections.ui_session_token)
}

#[rstest]
#[case::unset(&[], "24h")]
#[case::seven_days(&[(DURATION_ENV, "7d")], "7d")]
#[case::empty_stays_empty(&[(DURATION_ENV, "")], "")]
#[case::unparsed_text_loads(&[(DURATION_ENV, "nonsense")], "nonsense")]
fn loads_the_ui_session_duration(#[case] environment: &[(&str, &str)], #[case] duration: &str) {
    let settings = load(environment).unwrap();
    assert_eq!(settings.duration, duration);
}
