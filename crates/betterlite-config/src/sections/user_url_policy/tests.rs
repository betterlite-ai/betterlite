#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::sections::Section as _;
use crate::sections::user_url_policy::Raw;

fn load(yaml: &str, variables: &[(&str, &str)]) -> Option<(bool, Vec<String>)> {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let raw: Raw = serde_saphyr::from_str(yaml).unwrap();
    let settings = raw.convert(&environment).ok()?;
    Some((
        settings.user_url_validation,
        settings.user_url_allowed_hosts,
    ))
}

#[rstest]
#[case::defaults("{}", true)]
#[case::boolean_true("litellm_settings: {user_url_validation: true}", true)]
#[case::boolean_false("litellm_settings: {user_url_validation: false}", false)]
#[case::null_is_falsy("litellm_settings: {user_url_validation: null}", false)]
#[case::zero_is_falsy("litellm_settings: {user_url_validation: 0}", false)]
#[case::zero_float_is_falsy("litellm_settings: {user_url_validation: 0.0}", false)]
#[case::empty_string_is_falsy("litellm_settings: {user_url_validation: ''}", false)]
#[case::empty_list_is_falsy("litellm_settings: {user_url_validation: []}", false)]
#[case::empty_mapping_is_falsy("litellm_settings: {user_url_validation: {}}", false)]
#[case::string_false_is_truthy("litellm_settings: {user_url_validation: 'false'}", true)]
#[case::string_no_is_truthy("litellm_settings: {user_url_validation: 'no'}", true)]
#[case::one_is_truthy("litellm_settings: {user_url_validation: 1}", true)]
#[case::list_is_truthy("litellm_settings: {user_url_validation: [0]}", true)]
#[case::mapping_is_truthy("litellm_settings: {user_url_validation: {a: 1}}", true)]
#[case::general_false_beats_litellm_true(
    "litellm_settings: {user_url_validation: true}\ngeneral_settings: {user_url_validation: false}",
    false
)]
#[case::general_true_beats_litellm_false(
    "litellm_settings: {user_url_validation: false}\ngeneral_settings: {user_url_validation: true}",
    true
)]
#[case::general_null_changes_nothing(
    "litellm_settings: {user_url_validation: false}\ngeneral_settings: {user_url_validation: null}",
    false
)]
#[case::general_string_false_is_parsed("general_settings: {user_url_validation: ' False '}", false)]
#[case::general_string_true_is_parsed(
    "litellm_settings: {user_url_validation: false}\ngeneral_settings: {user_url_validation: 'TRUE'}",
    true
)]
#[case::general_unknown_string_changes_nothing(
    "litellm_settings: {user_url_validation: false}\ngeneral_settings: {user_url_validation: maybe}",
    false
)]
#[case::general_empty_string_changes_nothing("general_settings: {user_url_validation: ''}", true)]
#[case::general_zero_turns_off("general_settings: {user_url_validation: 0}", false)]
#[case::general_empty_list_turns_off("general_settings: {user_url_validation: []}", false)]
#[case::general_nonempty_list_turns_on(
    "litellm_settings: {user_url_validation: false}\ngeneral_settings: {user_url_validation: [a]}",
    true
)]
fn settings_follow_litellm_precedence(#[case] yaml: &str, #[case] expected: bool) {
    assert_eq!(load(yaml, &[]).unwrap().0, expected);
}

#[rstest]
#[case::absent("{}", &[])]
#[case::litellm_list("litellm_settings: {user_url_allowed_hosts: [a.test, 'b.test:8443']}", &["a.test", "b.test:8443"])]
#[case::litellm_null_is_empty("litellm_settings: {user_url_allowed_hosts: null}", &[])]
#[case::general_replaces(
    "litellm_settings: {user_url_allowed_hosts: [a.test]}\ngeneral_settings: {user_url_allowed_hosts: [b.test]}",
    &["b.test"]
)]
#[case::general_empty_list_replaces(
    "litellm_settings: {user_url_allowed_hosts: [a.test]}\ngeneral_settings: {user_url_allowed_hosts: []}",
    &[]
)]
#[case::general_null_replaces_with_empty(
    "litellm_settings: {user_url_allowed_hosts: [a.test]}\ngeneral_settings: {user_url_allowed_hosts: null}",
    &[]
)]
#[case::general_absent_keeps(
    "litellm_settings: {user_url_allowed_hosts: [a.test]}\ngeneral_settings: {user_url_validation: true}",
    &["a.test"]
)]
fn allowed_hosts_follow_litellm_precedence(#[case] yaml: &str, #[case] expected: &[&str]) {
    let hosts = load(yaml, &[]).unwrap().1;
    assert_eq!(hosts, expected);
}

#[test]
fn environment_references_resolve_like_the_rest_of_the_config() {
    let yaml = "litellm_settings:\n  user_url_validation: os.environ/VALIDATE\n  user_url_allowed_hosts: [os.environ/HOST]\ngeneral_settings:\n  user_url_validation: os.environ/GENERAL\n";
    let loaded = load(
        yaml,
        &[
            ("VALIDATE", "false"),
            ("HOST", "internal.test"),
            ("GENERAL", " no "),
        ],
    );
    assert_eq!(loaded, Some((true, vec!["internal.test".to_owned()])));
    let general_off = load(
        yaml,
        &[("VALIDATE", ""), ("HOST", "h"), ("GENERAL", "false")],
    );
    assert_eq!(general_off, Some((false, vec!["h".to_owned()])));
    let litellm_off = load(
        "litellm_settings: {user_url_validation: os.environ/VALIDATE}",
        &[("VALIDATE", "")],
    );
    assert_eq!(litellm_off, Some((false, Vec::new())));
    assert_eq!(load(yaml, &[("HOST", "h")]), None);
}
