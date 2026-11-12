#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

const DEFAULT_URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/main/litellm/anthropic_beta_headers_config.json";
const FLAG_ENV: &str = "LITELLM_LOCAL_ANTHROPIC_BETA_HEADERS";
const URL_ENV: &str = "LITELLM_ANTHROPIC_BETA_HEADERS_URL";

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .anthropic_beta_config)
}

#[rstest]
#[case::flag_omitted("", &[], false, DEFAULT_URL)]
#[case::flag_empty("", &[(FLAG_ENV, "")], false, DEFAULT_URL)]
#[case::flag_false("", &[(FLAG_ENV, "false")], false, DEFAULT_URL)]
#[case::flag_one("", &[(FLAG_ENV, "1")], false, DEFAULT_URL)]
#[case::flag_padded_true("", &[(FLAG_ENV, " true ")], false, DEFAULT_URL)]
#[case::flag_lowercase_true("", &[(FLAG_ENV, "true")], true, DEFAULT_URL)]
#[case::flag_uppercase_true("", &[(FLAG_ENV, "TRUE")], true, DEFAULT_URL)]
#[case::flag_title_case_true("", &[(FLAG_ENV, "True")], true, DEFAULT_URL)]
#[case::url_from_environment("", &[(URL_ENV, "http://beta.test/matrix.json")], false, "http://beta.test/matrix.json")]
#[case::url_empty_environment("", &[(URL_ENV, "")], false, "")]
#[case::url_settings_over_environment(
    "litellm_settings:\n  anthropic_beta_headers_url: http://yaml.test/matrix.json\n",
    &[(URL_ENV, "http://beta.test/matrix.json")],
    false,
    "http://yaml.test/matrix.json"
)]
#[case::url_empty_settings_over_environment(
    "litellm_settings:\n  anthropic_beta_headers_url: ''\n",
    &[(URL_ENV, "http://beta.test/matrix.json")],
    false,
    ""
)]
#[case::url_null_settings_keep_environment(
    "litellm_settings:\n  anthropic_beta_headers_url:\n",
    &[(URL_ENV, "http://beta.test/matrix.json")],
    false,
    "http://beta.test/matrix.json"
)]
fn loads_beta_settings(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] local_only: bool,
    #[case] url: &str,
) {
    let settings = load(source, environment).unwrap();
    assert_eq!(settings.local_only, local_only);
    assert_eq!(settings.url, url);
}

#[test]
fn rejects_a_url_of_the_wrong_type() {
    let error = load(
        "litellm_settings:\n  anthropic_beta_headers_url: [a]\n",
        &[],
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("anthropic_beta_headers_url"),
        "{error}"
    );
}
