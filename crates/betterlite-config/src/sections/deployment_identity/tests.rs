#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;

fn ignores(source: &str, variables: &[(&str, &str)]) -> Result<bool, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| {
            config
                .sections
                .deployment_identity
                .ignore_invalid_deployments
        })
        .map_err(|error| error.to_string())
}

#[rstest]
#[case::absent("model_list: []", &[], false)]
#[case::router_settings_without_the_key("router_settings: {}", &[], false)]
#[case::true_flag("router_settings:\n  ignore_invalid_deployments: true", &[], true)]
#[case::false_flag("router_settings:\n  ignore_invalid_deployments: false", &[], false)]
#[case::true_reference(
    "router_settings:\n  ignore_invalid_deployments: os.environ/IGNORE",
    &[("IGNORE", "true")],
    true
)]
#[case::false_reference(
    "router_settings:\n  ignore_invalid_deployments: os.environ/IGNORE",
    &[("IGNORE", "false")],
    false
)]
#[case::empty_reference_is_unset(
    "router_settings:\n  ignore_invalid_deployments: os.environ/IGNORE",
    &[("IGNORE", "")],
    false
)]
fn reads_ignore_invalid_deployments(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: bool,
) {
    assert_eq!(ignores(source, variables).unwrap(), expected);
}

#[rstest]
#[case::missing_reference(
    "router_settings:\n  ignore_invalid_deployments: os.environ/MISSING",
    &[]
)]
#[case::not_a_flag("router_settings:\n  ignore_invalid_deployments: maybe", &[])]
#[case::reference_not_a_flag(
    "router_settings:\n  ignore_invalid_deployments: os.environ/IGNORE",
    &[("IGNORE", "maybe")]
)]
#[case::number("router_settings:\n  ignore_invalid_deployments: 3", &[])]
fn rejects_an_invalid_ignore_flag(#[case] source: &str, #[case] variables: &[(&str, &str)]) {
    ignores(source, variables).unwrap_err();
}

#[test]
fn names_the_ignore_path() {
    let error = ignores("router_settings:\n  ignore_invalid_deployments: maybe", &[]).unwrap_err();
    assert!(
        error.contains("router_settings.ignore_invalid_deployments"),
        "{error}"
    );
}
