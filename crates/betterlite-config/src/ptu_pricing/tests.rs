#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{PTU_COST_ATTRIBUTION_ENV_VAR, load_ptu_cost_flag};

fn environment(value: Option<&str>) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::from([
        ("HOME".to_owned(), "/home/user".to_owned()),
        ("LITELLM_MASTER_KEY".to_owned(), "sk-test".to_owned()),
        ("PATH".to_owned(), "/usr/local/bin".to_owned()),
    ]);
    if let Some(text) = value {
        environment.insert(PTU_COST_ATTRIBUTION_ENV_VAR.to_owned(), text.to_owned());
    }
    environment
}

#[rstest]
#[case::absent(None, false)]
#[case::empty(Some(""), false)]
#[case::exact_true(Some("true"), true)]
#[case::upper_case(Some("TRUE"), true)]
#[case::padded(Some(" true "), true)]
#[case::exact_false(Some("false"), false)]
#[case::upper_false(Some("FALSE"), false)]
#[case::one(Some("1"), false)]
#[case::yes(Some("yes"), false)]
#[case::on(Some("on"), false)]
#[case::other(Some("enabled"), false)]
fn loads_ptu_cost_flag(#[case] value: Option<&str>, #[case] expected: bool) {
    assert_eq!(
        PTU_COST_ATTRIBUTION_ENV_VAR,
        "LITELLM_ENABLE_PTU_COST_ATTRIBUTION"
    );
    assert_eq!(load_ptu_cost_flag(&environment(value)), expected);
}

#[test]
fn loads_ptu_cost_flag_ignores_the_other_process_variables() {
    assert!(!load_ptu_cost_flag(&environment(None)));
}
