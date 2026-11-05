use super::{FakeTransport, run};

#[rstest::rstest]
#[case::match_searches_anywhere("regex_match", r#"["abc def","d.f"]"#, "{}", "true")]
#[case::match_without_a_hit("regex_match", r#"["abc","xyz"]"#, "{}", "false")]
#[case::match_ignore_case_flag("regex_match", r#"["ABC","abc",2]"#, "{}", "true")]
#[case::match_multiline_flag("regex_match", r#"["a\nb","^b$",8]"#, "{}", "true")]
#[case::match_dot_all_flag("regex_match", r#"["a\nb","a.b",16]"#, "{}", "true")]
#[case::match_named_flags("regex_match", r#"["ABC","abc"]"#, r#"{"flags":2}"#, "true")]
#[case::match_all_whole_text("regex_match_all", r#"["abc","abc"]"#, "{}", "true")]
#[case::match_all_rejects_a_prefix("regex_match_all", r#"["abcd","abc"]"#, "{}", "false")]
#[case::match_all_alternation_is_grouped("regex_match_all", r#"["b","a|b"]"#, "{}", "true")]
#[case::replace_every_hit("regex_replace", r##"["a1b22","\\d+","#"]"##, "{}", r#""a#b#""#)]
#[case::replace_with_group_references(
    "regex_replace",
    r#"["john smith","(\\w+) (\\w+)","\\2 \\1"]"#,
    "{}",
    r#""smith john""#
)]
#[case::replace_with_named_group(
    "regex_replace",
    r#"["ab","(?P<x>a)","[\\g<x>]"]"#,
    "{}",
    r#""[a]b""#
)]
#[case::find_all_without_groups("regex_find_all", r#"["a1b22","\\d+"]"#, "{}", r#"["1","22"]"#)]
#[case::find_all_one_group("regex_find_all", r#"["k=1 j=2","(\\w)="]"#, "{}", r#"["k","j"]"#)]
#[case::find_all_many_groups(
    "regex_find_all",
    r#"["k=1 j=2","(\\w)=(\\d)"]"#,
    "{}",
    r#"[["k","1"],["j","2"]]"#
)]
#[case::json_parse_object(
    "json_parse",
    r#"["{\"a\":[1,true,null]}"]"#,
    "{}",
    r#"{"a":[1,true,null]}"#
)]
#[case::json_stringify_object("json_stringify", r#"[{"a":[1,2]}]"#, "{}", r#""{\"a\":[1,2]}""#)]
#[case::json_stringify_null("json_stringify", "[null]", "{}", r#""null""#)]
#[case::schema_accepts_a_conforming_object(
    "json_schema_valid",
    r#"[{"name":"x","age":3},{"type":"object","required":["name"],"properties":{"age":{"type":"integer"}}}]"#,
    "{}",
    "true"
)]
#[case::schema_rejects_a_missing_required_key(
    "json_schema_valid",
    r#"[{"age":3},{"type":"object","required":["name"]}]"#,
    "{}",
    "false"
)]
#[case::schema_rejects_a_wrong_nested_type(
    "json_schema_valid",
    r#"[{"age":"3"},{"type":"object","properties":{"age":{"type":"integer"}}}]"#,
    "{}",
    "false"
)]
#[case::schema_number_accepts_a_float(
    "json_schema_valid",
    r#"[1.5,{"type":"number"}]"#,
    "{}",
    "true"
)]
#[case::schema_integer_rejects_a_float(
    "json_schema_valid",
    r#"[1.5,{"type":"integer"}]"#,
    "{}",
    "false"
)]
#[case::extract_urls_finds_both_schemes(
    "extract_urls",
    r#"["see https://a.com/x and http://b.org."]"#,
    "{}",
    r#"["https://a.com/x","http://b.org."]"#
)]
#[case::extract_urls_ignores_other_schemes(
    "extract_urls",
    r#"["ftp://a.com no urls"]"#,
    "{}",
    "[]"
)]
#[case::is_valid_url_full("is_valid_url", r#"["https://example.com/p"]"#, "{}", "true")]
#[case::is_valid_url_without_scheme("is_valid_url", r#"["example.com"]"#, "{}", "false")]
#[case::is_valid_url_without_host("is_valid_url", r#"["http://"]"#, "{}", "false")]
#[case::domain_keeps_the_port(
    "get_url_domain",
    r#"["https://example.com:8080/x"]"#,
    "{}",
    r#""example.com:8080""#
)]
#[case::domain_of_a_non_url("get_url_domain", r#"["mailto:a@b.c"]"#, "{}", "null")]
#[case::all_urls_valid_without_urls("all_urls_valid", r#"["no urls here"]"#, "{}", "true")]
#[case::all_urls_valid_with_one_url("all_urls_valid", r#"["https://example.com"]"#, "{}", "true")]
#[case::all_urls_valid_rejects_a_broken_host(
    "all_urls_valid",
    r#"["go to http://a[b now"]"#,
    "{}",
    "false"
)]
#[tokio::test]
async fn primitives_management_valid(
    #[case] name: &str,
    #[case] positional: &str,
    #[case] named: &str,
    #[case] expected: &str,
) {
    let output = run(FakeTransport::default(), name, positional, named)
        .await
        .expect("the call succeeds");
    assert_eq!(output, expected);
}
