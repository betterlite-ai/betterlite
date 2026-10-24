use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

#[rstest]
#[case::loads_default_max_tokens("", &[], Ok((false, 4096)))]
#[case::loads_default_max_tokens("", &[("DEFAULT_MAX_TOKENS", " +100 ")], Ok((false, 100)))]
#[case::loads_default_max_tokens("", &[("DEFAULT_MAX_TOKENS", "4k")], Err("DEFAULT_MAX_TOKENS"))]
#[case::loads_default_max_tokens(
    "litellm_settings:\n  DEFAULT_MAX_TOKENS: 5\n",
    &[],
    Err("litellm_settings.DEFAULT_MAX_TOKENS")
)]
#[case::loads_modify_params("litellm_settings:\n  modify_params: true\n", &[], Ok((true, 4096)))]
#[case::loads_modify_params(
    "litellm_settings:\n  modify_params: false\n",
    &[("LITELLM_MODIFY_PARAMS", "1")],
    Ok((false, 4096))
)]
#[case::loads_modify_params("litellm_settings:\n  modify_params: 'false'\n", &[], Ok((false, 4096)))]
#[case::loads_modify_params("litellm_settings:\n  modify_params: [true]\n", &[], Err("yaml"))]
#[case::loads_litellm_modify_params("", &[("LITELLM_MODIFY_PARAMS", "false")], Ok((true, 4096)))]
#[case::loads_litellm_modify_params("", &[("LITELLM_MODIFY_PARAMS", "")], Ok((false, 4096)))]
#[case::loads_litellm_modify_params("", &[("LITELLM_MODIFY_PARAMS", " ")], Ok((true, 4096)))]
fn provider_base_config_cases(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Result<(bool, i64), &str>,
) {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let loaded = ConfigLoader { lookup }.load(source);
    match (loaded, expected) {
        (Ok(config), Ok(settings)) => {
            let litellm = config.litellm_settings;
            assert_eq!((litellm.modify_params, litellm.default_max_tokens), settings);
        }
        (Err(ConfigError::InvalidInteger { path, .. }), Err(expected_path)) => {
            assert_eq!(path, expected_path);
        }
        (Err(ConfigError::Unsupported { paths }), Err(expected_path)) => {
            assert_eq!(paths, [expected_path]);
        }
        (Err(ConfigError::Yaml { .. }), Err("yaml")) => {}
        (outcome, expectation) => panic!("{outcome:?} does not match {expectation:?}"),
    }
}
