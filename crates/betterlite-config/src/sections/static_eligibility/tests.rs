#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;

fn loaded(source: &str) -> bool {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .static_eligibility
    .enable_pre_call_checks
}

#[rstest]
#[case::absent("router_settings: {}\n", false)]
#[case::off("router_settings:\n  enable_pre_call_checks: false\n", false)]
#[case::on("router_settings:\n  enable_pre_call_checks: true\n", true)]
fn static_eligibility_enable_pre_call_checks_loads(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(loaded(source), expected);
}

#[test]
fn static_eligibility_defaults_when_router_settings_are_absent() {
    assert!(!loaded("model_list: []\n"));
}
