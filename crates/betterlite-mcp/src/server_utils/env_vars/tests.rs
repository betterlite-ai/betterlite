#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::mcp_stored_contracts::JsonValue;
use rstest::rstest;

use super::{
    MCPMissingUserEnvVarsError, UserEnvSpec, build_env_var_setup_url, collect_env_var_references,
    find_env_var_references, interpolate_env_vars, interpolate_headers, parse_admin_env_vars,
};

fn value(json: &str) -> JsonValue {
    serde_json::from_str(json).unwrap()
}

fn entries(json: &str) -> Vec<JsonValue> {
    serde_json::from_str(json).unwrap()
}

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn variables(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn spec(name: &str, description: Option<&str>) -> UserEnvSpec {
    UserEnvSpec {
        name: name.to_owned(),
        description: description.map(str::to_owned),
    }
}

#[test]
fn parse_admin_env_vars_returns_nothing_without_entries() {
    let empty = parse_admin_env_vars(None);
    assert_eq!(empty, (BTreeMap::new(), Vec::new()));
    assert_eq!(
        parse_admin_env_vars(Some(&[])),
        (BTreeMap::new(), Vec::new())
    );
}

#[test]
fn parse_admin_env_vars_splits_globals_from_user_specs() {
    let env_vars = entries(
        r#"[
            {"name": "TOKEN", "value": "abc"},
            {"name": "OWNER", "scope": "user", "description": "your owner"},
            {"name": "REGION", "scope": "global", "value": "eu"}
        ]"#,
    );
    let (globals, specs) = parse_admin_env_vars(Some(&env_vars));
    assert_eq!(globals, variables(&[("REGION", "eu"), ("TOKEN", "abc")]));
    assert_eq!(specs, vec![spec("OWNER", Some("your owner"))]);
}

#[test]
fn parse_admin_env_vars_preserves_the_user_spec_order() {
    let env_vars = entries(
        r#"[
            {"name": "B", "scope": "user"},
            {"name": "A", "scope": "user"}
        ]"#,
    );
    let (_, specs) = parse_admin_env_vars(Some(&env_vars));
    assert_eq!(specs, vec![spec("B", None), spec("A", None)]);
}

#[rstest]
#[case::missing_scope(r#"{"name": "A", "value": "1"}"#)]
#[case::empty_scope(r#"{"name": "A", "scope": "", "value": "1"}"#)]
#[case::null_scope(r#"{"name": "A", "scope": null, "value": "1"}"#)]
#[case::global_scope(r#"{"name": "A", "scope": "global", "value": "1"}"#)]
#[case::unknown_scope(r#"{"name": "A", "scope": "team", "value": "1"}"#)]
#[case::numeric_scope(r#"{"name": "A", "scope": 5, "value": "1"}"#)]
#[case::mixed_case_scope(r#"{"name": "A", "scope": "User", "value": "1"}"#)]
fn parse_admin_env_vars_treats_every_other_scope_as_global(#[case] entry: &str) {
    let env_vars = vec![value(entry)];
    let (globals, specs) = parse_admin_env_vars(Some(&env_vars));
    assert_eq!(globals, variables(&[("A", "1")]));
    assert!(specs.is_empty());
}

#[rstest]
#[case::null(r#"{"name": "A", "value": null}"#, "")]
#[case::missing(r#"{"name": "A"}"#, "")]
#[case::text(r#"{"name": "A", "value": "text"}"#, "text")]
#[case::empty_text(r#"{"name": "A", "value": ""}"#, "")]
#[case::number(r#"{"name": "A", "value": 5}"#, "5")]
#[case::float(r#"{"name": "A", "value": 1.5}"#, "1.5")]
#[case::boolean(r#"{"name": "A", "value": true}"#, "true")]
#[case::array(r#"{"name": "A", "value": [1, "two"]}"#, r#"[1,"two"]"#)]
#[case::object(r#"{"name": "A", "value": {"k": 1}}"#, r#"{"k":1}"#)]
fn parse_admin_env_vars_writes_the_stored_value_text(#[case] entry: &str, #[case] expected: &str) {
    let (globals, _) = parse_admin_env_vars(Some(&[value(entry)]));
    assert_eq!(globals.get("A").map(String::as_str), Some(expected));
}

#[test]
fn parse_admin_env_vars_lets_a_later_global_replace_an_earlier_one() {
    let env_vars = entries(
        r#"[{"name": "A", "value": "first"}, {"name": "A", "scope": "global", "value": "second"}]"#,
    );
    let (globals, _) = parse_admin_env_vars(Some(&env_vars));
    assert_eq!(globals, variables(&[("A", "second")]));
}

#[test]
fn parse_admin_env_vars_keeps_a_global_when_a_user_spec_follows() {
    let env_vars = entries(
        r#"[{"name": "A", "value": "1"}, {"name": "A", "scope": "user", "description": "d"}]"#,
    );
    let (globals, specs) = parse_admin_env_vars(Some(&env_vars));
    assert_eq!(globals, variables(&[("A", "1")]));
    assert_eq!(specs, vec![spec("A", Some("d"))]);
}

#[test]
fn parse_admin_env_vars_reads_only_a_string_description() {
    let env_vars = entries(
        r#"[
            {"name": "A", "scope": "user", "description": null},
            {"name": "B", "scope": "user", "description": 5},
            {"name": "C", "scope": "user", "description": ""}
        ]"#,
    );
    let (_, specs) = parse_admin_env_vars(Some(&env_vars));
    assert_eq!(
        specs,
        vec![spec("A", None), spec("B", None), spec("C", Some(""))]
    );
}

#[rstest]
#[case::null_entry("null")]
#[case::text_entry(r#""A""#)]
#[case::number_entry("5")]
#[case::array_entry(r#"["A"]"#)]
#[case::missing_name(r#"{"value": "1"}"#)]
#[case::null_name(r#"{"name": null, "value": "1"}"#)]
#[case::numeric_name(r#"{"name": 5, "value": "1"}"#)]
#[case::empty_name(r#"{"name": "", "value": "1"}"#)]
fn parse_admin_env_vars_skips_a_malformed_entry(#[case] entry: &str) {
    let env_vars = vec![value(entry)];
    assert_eq!(
        parse_admin_env_vars(Some(&env_vars)),
        (BTreeMap::new(), Vec::new())
    );
}

#[test]
fn find_env_var_references_reads_well_formed_references() {
    assert_eq!(find_env_var_references("${A}"), names(&["A"]));
    assert_eq!(
        find_env_var_references("Bearer ${API_TOKEN}/x"),
        names(&["API_TOKEN"])
    );
    assert_eq!(
        find_env_var_references("${_A1}${a}${A1_}"),
        names(&["_A1", "A1_", "a"])
    );
}

#[test]
fn find_env_var_references_returns_a_set() {
    assert_eq!(find_env_var_references("${A}${B}${A}"), names(&["A", "B"]));
}

#[test]
fn find_env_var_references_returns_nothing_for_an_empty_text() {
    assert!(find_env_var_references("").is_empty());
}

#[rstest]
#[case::bare_name("$NAME")]
#[case::digit_first("${1A}")]
#[case::hyphen("${A-B}")]
#[case::space("${A B}")]
#[case::unclosed("${A")]
#[case::no_braces("$A$B")]
#[case::non_ascii("${Ü}")]
#[case::dollar_after_brace("${A$B}")]
fn find_env_var_references_ignores_other_dollar_forms(#[case] text: &str) {
    assert!(find_env_var_references(text).is_empty());
}

#[test]
fn collect_env_var_references_unions_every_string() {
    let strings = ["${A}", "x${B}y", "${A}${C}"];
    assert_eq!(collect_env_var_references(strings), names(&["A", "B", "C"]));
}

#[test]
fn collect_env_var_references_accepts_an_empty_collection() {
    let strings: [&str; 0] = [];
    assert!(collect_env_var_references(strings).is_empty());
}

#[test]
fn interpolate_env_vars_replaces_a_known_name() {
    let known = variables(&[("A", "value")]);
    assert_eq!(interpolate_env_vars("${A}", &known), "value");
    assert_eq!(interpolate_env_vars("x${A}y", &known), "xvaluey");
}

#[test]
fn interpolate_env_vars_replaces_every_occurrence() {
    let known = variables(&[("A", "1")]);
    assert_eq!(interpolate_env_vars("${A}-${A}", &known), "1-1");
}

#[test]
fn interpolate_env_vars_leaves_an_unknown_reference_untouched() {
    let known = variables(&[("A", "1")]);
    assert_eq!(interpolate_env_vars("${OTHER}", &known), "${OTHER}");
    assert_eq!(interpolate_env_vars("${A}-${OTHER}", &known), "1-${OTHER}");
}

#[test]
fn interpolate_env_vars_makes_one_pass() {
    let known = variables(&[("A", "${B}"), ("B", "expanded")]);
    assert_eq!(interpolate_env_vars("${A}", &known), "${B}");
}

#[test]
fn interpolate_env_vars_keeps_text_the_pattern_does_not_match() {
    let known = variables(&[("A", "1")]);
    assert_eq!(interpolate_env_vars("${1A} $A ${A", &known), "${1A} $A ${A");
}

#[test]
fn interpolate_env_vars_returns_an_empty_text_as_is() {
    assert_eq!(interpolate_env_vars("", &variables(&[("A", "1")])), "");
}

#[test]
fn interpolate_headers_keeps_the_keys_and_interpolates_the_values() {
    let headers = BTreeMap::from([
        ("Authorization".to_owned(), "Bearer ${TOKEN}".to_owned()),
        ("X-Other".to_owned(), "${UNKNOWN}".to_owned()),
    ]);
    let known = variables(&[("TOKEN", "secret")]);
    assert_eq!(
        interpolate_headers(&headers, &known),
        BTreeMap::from([
            ("Authorization".to_owned(), "Bearer secret".to_owned()),
            ("X-Other".to_owned(), "${UNKNOWN}".to_owned()),
        ])
    );
}

#[test]
fn interpolate_headers_returns_an_empty_map() {
    assert!(interpolate_headers(&BTreeMap::new(), &variables(&[("A", "1")])).is_empty());
}

#[rstest]
#[case::no_base(None, "/ui/mcp-servers?fill_env_vars=srv")]
#[case::empty_base(Some(""), "/ui/mcp-servers?fill_env_vars=srv")]
#[case::base(
    Some("https://proxy.test"),
    "https://proxy.test/ui/mcp-servers?fill_env_vars=srv"
)]
#[case::trailing_slash(
    Some("https://proxy.test/"),
    "https://proxy.test/ui/mcp-servers?fill_env_vars=srv"
)]
#[case::trailing_slashes(
    Some("https://proxy.test///"),
    "https://proxy.test/ui/mcp-servers?fill_env_vars=srv"
)]
#[case::base_path(
    Some("https://proxy.test/base"),
    "https://proxy.test/base/ui/mcp-servers?fill_env_vars=srv"
)]
fn build_env_var_setup_url_joins_the_base_and_the_path(
    #[case] base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(build_env_var_setup_url(base, "srv"), expected);
}

#[test]
fn build_env_var_setup_url_encodes_a_slash_in_the_server_id() {
    assert_eq!(
        build_env_var_setup_url(Some("https://proxy.test"), "srv/1"),
        "https://proxy.test/ui/mcp-servers?fill_env_vars=srv%2F1"
    );
}

#[rstest]
#[case::space("srv 1", "srv%201")]
#[case::plus("a+b", "a%2Bb")]
#[case::star("a*b", "a%2Ab")]
#[case::at("a@b", "a%40b")]
#[case::kept("a_b.c-d~e", "a_b.c-d~e")]
#[case::unicode("é", "%C3%A9")]
#[case::empty("", "")]
fn build_env_var_setup_url_encodes_the_server_id_like_python_quote(
    #[case] server_id: &str,
    #[case] encoded: &str,
) {
    assert_eq!(
        build_env_var_setup_url(None, server_id),
        format!("/ui/mcp-servers?fill_env_vars={encoded}")
    );
}

fn missing_error(server_name: Option<&str>, missing: &[&str]) -> MCPMissingUserEnvVarsError {
    MCPMissingUserEnvVarsError {
        server_id: "srv".to_owned(),
        server_name: server_name.map(str::to_owned),
        missing: missing.iter().map(|name| (*name).to_owned()).collect(),
        setup_url: "https://proxy.test/ui/mcp-servers?fill_env_vars=srv".to_owned(),
    }
}

#[test]
fn missing_user_env_vars_error_names_one_variable() {
    let error = missing_error(Some("Pets"), &["TOKEN"]);
    assert_eq!(
        error.to_string(),
        "Cannot connect to MCP server \"Pets\".\n\nYour administrator configured this server to require per-user variables, but you haven't set the following yet:\n- TOKEN\n\nSet your credentials here:\nhttps://proxy.test/ui/mcp-servers?fill_env_vars=srv"
    );
}

#[test]
fn missing_user_env_vars_error_lists_several_variables() {
    let error = missing_error(None, &["TOKEN", "OWNER"]);
    assert_eq!(
        error.to_string(),
        "Cannot connect to MCP server \"srv\".\n\nYour administrator configured this server to require per-user variables, but you haven't set the following yet:\n- TOKEN\n- OWNER\n\nSet your credentials here:\nhttps://proxy.test/ui/mcp-servers?fill_env_vars=srv"
    );
}

#[test]
fn missing_user_env_vars_error_falls_back_to_the_server_id() {
    let unnamed = missing_error(None, &["TOKEN"]);
    assert!(unnamed.to_string().contains("MCP server \"srv\"."));
    let empty_name = missing_error(Some(""), &["TOKEN"]);
    assert_eq!(empty_name.to_string(), unnamed.to_string());
}
