#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use super::{
    add_server_prefix_to_name, is_tool_name_prefixed, known_tool_name_spellings,
    match_known_tool_name, openapi_tool_name, short_tool_prefix_enabled,
};

fn prefixes(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn add_server_prefix_to_name_normalizes_only_the_server_name() {
    assert_eq!(
        add_server_prefix_to_name("get pets", "My Server", "-"),
        "My_Server-get pets"
    );
}

#[test]
fn add_server_prefix_to_name_joins_with_the_given_separator() {
    assert_eq!(
        add_server_prefix_to_name("get_pets", "srv", "."),
        "srv.get_pets"
    );
    assert_eq!(
        add_server_prefix_to_name("get_pets", "srv", ""),
        "srvget_pets"
    );
}

#[rstest]
#[case::one("1")]
#[case::true_lower("true")]
#[case::true_upper("TRUE")]
#[case::yes("yes")]
#[case::on("on")]
#[case::padded("  Yes  ")]
fn short_tool_prefix_enabled_accepts_the_documented_spellings(#[case] raw: &str) {
    assert!(short_tool_prefix_enabled(Some(raw)));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[case::blank(Some("   "))]
#[case::zero(Some("0"))]
#[case::no(Some("no"))]
#[case::off(Some("off"))]
#[case::two(Some("2"))]
#[case::word(Some("enable"))]
fn short_tool_prefix_enabled_rejects_every_other_value(#[case] raw: Option<&str>) {
    assert!(!short_tool_prefix_enabled(raw));
}

#[test]
fn known_tool_name_spellings_lists_the_bare_name_first() {
    assert_eq!(
        known_tool_name_spellings("get_pets", &prefixes(&["srv", "Server Name"]), "-"),
        vec![
            "get_pets".to_owned(),
            "srv-get_pets".to_owned(),
            "Server_Name-get_pets".to_owned(),
        ]
    );
}

#[test]
fn known_tool_name_spellings_returns_only_the_bare_name_without_prefixes() {
    assert_eq!(
        known_tool_name_spellings("get_pets", &[], "-"),
        vec!["get_pets".to_owned()]
    );
}

#[test]
fn openapi_tool_name_replaces_spaces_and_lowercases() {
    assert_eq!(openapi_tool_name("Get Pets"), "get_pets");
    assert_eq!(openapi_tool_name("get pets_v2"), "get_pets_v2");
    assert_eq!(openapi_tool_name("get_pets"), "get_pets");
    assert_eq!(openapi_tool_name(""), "");
}

#[test]
fn match_known_tool_name_finds_the_bare_name() {
    let names = ["get_pets", "get_owners"];
    assert_eq!(
        match_known_tool_name("get_pets", false, &prefixes(&["srv"]), "-", names),
        Some("get_pets")
    );
}

#[test]
fn match_known_tool_name_finds_every_prefixed_spelling() {
    let names = ["srv-get_pets"];
    assert_eq!(
        match_known_tool_name("get_pets", false, &prefixes(&["srv"]), "-", names),
        Some("srv-get_pets")
    );
    let second = ["Server_Name-get_pets"];
    assert_eq!(
        match_known_tool_name(
            "get_pets",
            false,
            &prefixes(&["srv", "Server Name"]),
            "-",
            second
        ),
        Some("Server_Name-get_pets")
    );
}

#[test]
fn match_known_tool_name_returns_the_entry_as_written() {
    let names = ["Srv-Get_Pets"];
    assert_eq!(
        match_known_tool_name("Get_Pets", false, &prefixes(&["Srv"]), "-", names),
        Some("Srv-Get_Pets")
    );
}

#[test]
fn match_known_tool_name_compares_exactly_without_openapi() {
    let names = ["Get_Pets", "get_pets"];
    assert_eq!(
        match_known_tool_name("GET_PETS", false, &prefixes(&["srv"]), "-", names),
        None
    );
}

#[test]
fn match_known_tool_name_normalizes_both_sides_for_an_openapi_server() {
    let configured = ["Get Pets"];
    assert_eq!(
        match_known_tool_name("get_pets", true, &[], "-", configured),
        Some("Get Pets")
    );
    let served = ["get_pets"];
    assert_eq!(
        match_known_tool_name("Get Pets", true, &[], "-", served),
        Some("get_pets")
    );
}

#[test]
fn match_known_tool_name_normalizes_prefixed_spellings_for_an_openapi_server() {
    let configured = ["Srv-Get Pets"];
    assert_eq!(
        match_known_tool_name("get_pets", true, &prefixes(&["srv"]), "-", configured),
        Some("Srv-Get Pets")
    );
}

#[test]
fn match_known_tool_name_returns_none_for_an_empty_name_list() {
    let names: [&str; 0] = [];
    assert_eq!(
        match_known_tool_name("get_pets", false, &prefixes(&["srv"]), "-", names),
        None
    );
}

#[test]
fn is_tool_name_prefixed_is_false_without_the_separator() {
    let known = BTreeSet::from(["srv".to_owned()]);
    assert!(!is_tool_name_prefixed("get_pets", Some(&known), "-"));
}

#[test]
fn is_tool_name_prefixed_falls_back_to_the_legacy_rule_without_a_set() {
    assert!(is_tool_name_prefixed("text-to-speech", None, "-"));
    assert!(!is_tool_name_prefixed("get_pets", None, "-"));
}

#[test]
fn is_tool_name_prefixed_checks_the_known_prefixes_when_given() {
    let known = BTreeSet::from(["srv".to_owned(), "Server Name".to_owned()]);
    assert!(is_tool_name_prefixed("srv-get_pets", Some(&known), "-"));
    assert!(is_tool_name_prefixed(
        "Server_Name-get_pets",
        Some(&known),
        "-"
    ));
    assert!(!is_tool_name_prefixed("text-to-speech", Some(&known), "-"));
    assert!(!is_tool_name_prefixed("other-get_pets", Some(&known), "-"));
}

#[test]
fn is_tool_name_prefixed_uses_the_given_separator() {
    let known = BTreeSet::from(["srv".to_owned()]);
    assert!(is_tool_name_prefixed("srv.get_pets", Some(&known), "."));
    assert!(!is_tool_name_prefixed("srv-get_pets", Some(&known), "."));
    assert!(!is_tool_name_prefixed("srv.get_pets", Some(&known), "-"));
}

#[test]
fn is_tool_name_prefixed_accepts_a_prefix_that_contains_the_separator() {
    let known = BTreeSet::from(["abcdef12-3456-7890-abcd-ef1234567890".to_owned()]);
    assert!(is_tool_name_prefixed(
        "abcdef12-3456-7890-abcd-ef1234567890-get_pets",
        Some(&known),
        "-"
    ));
    assert!(!is_tool_name_prefixed(
        "abcdef12-3456-7890-abcd-ef1234567890",
        Some(&known),
        "-"
    ));
}
