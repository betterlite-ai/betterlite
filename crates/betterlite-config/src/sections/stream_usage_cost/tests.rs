#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(yaml: &str, variables: &[(&str, &str)]) -> Result<bool, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(yaml)
        .map(|config| config.sections.stream_usage_cost.enabled)
}

#[rstest]
#[case::enabled("litellm_settings: {include_cost_in_streaming_usage: true}", &[], true)]
#[case::disabled("litellm_settings: {include_cost_in_streaming_usage: false}", &[], false)]
#[case::absent("litellm_settings: {}", &[], false)]
#[case::null_takes_the_default("litellm_settings: {include_cost_in_streaming_usage: null}", &[], false)]
#[case::no_litellm_settings("model_list: []", &[], false)]
#[case::true_text("litellm_settings: {include_cost_in_streaming_usage: \"true\"}", &[], true)]
#[case::yes_text("litellm_settings: {include_cost_in_streaming_usage: \"yes\"}", &[], true)]
#[case::off_text("litellm_settings: {include_cost_in_streaming_usage: \"off\"}", &[], false)]
#[case::environment_reference(
    "litellm_settings: {include_cost_in_streaming_usage: os.environ/INCLUDE_COST_IN_STREAMING_USAGE}",
    &[("INCLUDE_COST_IN_STREAMING_USAGE", "yes")],
    true
)]
fn the_flag_loads_through_the_real_loader(
    #[case] yaml: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: bool,
) {
    assert_eq!(load(yaml, variables).unwrap(), expected);
}

#[rstest]
#[case::number("litellm_settings: {include_cost_in_streaming_usage: 3}")]
#[case::list("litellm_settings: {include_cost_in_streaming_usage: [true]}")]
#[case::mapping("litellm_settings: {include_cost_in_streaming_usage: {flag: true}}")]
fn a_non_scalar_value_is_a_load_error(#[case] yaml: &str) {
    assert!(matches!(load(yaml, &[]), Err(ConfigError::Yaml { .. })));
}

#[test]
fn text_that_is_not_a_flag_is_a_load_error() {
    assert!(matches!(
        load(
            "litellm_settings: {include_cost_in_streaming_usage: maybe}",
            &[]
        ),
        Err(ConfigError::Environment { .. })
    ));
}

#[test]
fn a_missing_environment_variable_names_its_path() {
    let error = load(
        "litellm_settings: {include_cost_in_streaming_usage: os.environ/ABSENT_FLAG}",
        &[],
    )
    .unwrap_err();
    let ConfigError::MissingEnvironment { variable, path } = error else {
        panic!("a missing reference names the variable");
    };
    assert_eq!(variable, "ABSENT_FLAG");
    assert_eq!(path, "litellm_settings.include_cost_in_streaming_usage");
}

#[test]
fn an_unrelated_litellm_setting_is_still_unsupported() {
    let error = load(
        "litellm_settings: {include_cost_in_streaming_usage: true, unrelated_stream_cost: true}",
        &[],
    )
    .unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("an unrelated key is unsupported");
    };
    assert_eq!(
        paths,
        vec!["litellm_settings.unrelated_stream_cost".to_owned()]
    );
}
