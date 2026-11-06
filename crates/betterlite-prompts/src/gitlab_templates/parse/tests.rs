#![cfg(test)]

use rstest::rstest;

use super::{FrontmatterParser, ParseError, parse_prompt_file, parse_yaml_basic};

fn parse_yaml(content: &str) -> Result<super::GitLabPromptTemplate, ParseError> {
    parse_prompt_file(content, "id", FrontmatterParser::Yaml)
}

fn metadata_json(content: &str) -> String {
    serde_json::to_string(&parse_yaml(content).unwrap().metadata).unwrap()
}

#[rstest]
#[case::frontmatter_is_split_and_trimmed(
    "---\nmodel: gpt-4\n---\n\n  Hello {{ name }}  \n",
    "Hello {{ name }}"
)]
#[case::no_frontmatter_keeps_the_content("  Hello  \n", "  Hello  \n")]
#[case::unclosed_fence_keeps_the_whole_file("---\nmodel: gpt-4\nHello", "---\nmodel: gpt-4\nHello")]
#[case::fence_not_at_the_start(
    "Hello\n---\nmodel: gpt-4\n---\nWorld",
    "Hello\n---\nmodel: gpt-4\n---\nWorld"
)]
#[case::later_fences_stay_in_the_body("---\na: 1\n---\nbody\n---\nmore", "body\n---\nmore")]
#[case::fence_on_one_line("---a: 1---body", "body")]
#[case::empty_frontmatter("------\nbody", "body")]
fn the_template_content_follows_the_frontmatter_split(
    #[case] content: &str,
    #[case] expected: &str,
) {
    assert_eq!(parse_yaml(content).unwrap().content, expected);
}

#[rstest]
#[case::mapping(
    "---\nmodel: gpt-4\ntemperature: 0.5\nstop: [a, b]\n---\nx",
    r#"{"model":"gpt-4","temperature":0.5,"stop":["a","b"]}"#
)]
#[case::nested(
    "---\ninput:\n  schema:\n    name: string\n---\nx",
    r#"{"input":{"schema":{"name":"string"}}}"#
)]
#[case::last_duplicate_key_wins("---\na: 1\nb: 2\na: 3\n---\nx", r#"{"a":3,"b":2}"#)]
#[case::no_frontmatter("plain", "{}")]
#[case::invalid_yaml_is_swallowed("---\n: : [unclosed\n---\nx", "{}")]
#[case::tabs_are_invalid_yaml("---\nkey:\n\t- a\n---\nx", "{}")]
#[case::null_document("---\nnull\n---\nx", "{}")]
#[case::empty_list_document("---\n[]\n---\nx", "{}")]
#[case::zero_document("---\n0\n---\nx", "{}")]
#[case::comment_only("---\n# nothing\n---\nx", "{}")]
#[case::non_text_keys_are_a_deviation_and_leave_the_metadata_empty(
    "---\n1: a\nmodel: m\n---\nx",
    "{}"
)]
#[case::a_plain_n_key_stays_text(
    "---\nmodel: gpt-4\nn: 2\nstop: n\nflag: y\n---\nx",
    r#"{"model":"gpt-4","n":2,"stop":"n","flag":"y"}"#
)]
#[case::yaml11_bool_spellings_are_booleans(
    "---\na: yes\nb: Yes\nc: YES\nd: on\ne: On\nf: ON\ng: no\nh: No\ni: NO\nj: off\nk: Off\nl: OFF\n---\nx",
    r#"{"a":true,"b":true,"c":true,"d":true,"e":true,"f":true,"g":false,"h":false,"i":false,"j":false,"k":false,"l":false}"#
)]
#[case::other_bool_casings_stay_text(
    "---\na: tRuE\nb: yEs\nc: oN\nd: fAlSe\n---\nx",
    r#"{"a":"tRuE","b":"yEs","c":"oN","d":"fAlSe"}"#
)]
#[case::nested_lists_and_maps_resolve_bools(
    "---\nstop: [yes, no, y]\ninput:\n  schema:\n    strict: off\n---\nx",
    r#"{"stop":[true,false,"y"],"input":{"schema":{"strict":false}}}"#
)]
#[case::quoted_bool_spellings_resolve_too(
    "---\na: \"yes\"\nb: 'on'\n---\nx",
    r#"{"a":true,"b":true}"#
)]
fn yaml_frontmatter_becomes_the_metadata(#[case] content: &str, #[case] expected: &str) {
    assert_eq!(metadata_json(content), expected);
}

#[rstest]
#[case::list("---\n- a\n- b\n---\nx")]
#[case::text("---\njust text\n---\nx")]
#[case::number("---\n42\n---\nx")]
fn a_truthy_document_that_is_not_a_mapping_is_an_error(#[case] content: &str) {
    assert!(matches!(
        parse_yaml(content),
        Err(ParseError::MetadataNotMapping)
    ));
}

#[test]
fn metadata_that_fails_the_template_checks_is_an_error() {
    let outcome = parse_yaml("---\nmodel: [a]\n---\nx");
    assert!(matches!(outcome, Err(ParseError::Template(_))));
}

#[test]
fn the_basic_parser_reads_flat_key_value_lines() {
    let content = "---\nmodel: gpt-4\ntemperature: 0.5\n  nested: 1\n---\nBody";
    let template = parse_prompt_file(content, "id", FrontmatterParser::Basic).unwrap();
    assert_eq!(
        serde_json::to_string(&template.metadata).unwrap(),
        r#"{"model":"gpt-4","temperature":0.5,"nested":1}"#
    );
    assert_eq!(template.content, "Body");
    assert_eq!(template.model.as_deref(), Some("gpt-4"));
}

#[test]
fn the_basic_parser_keeps_a_yaml_list_as_text() {
    let content = "---\nstop: [a, b]\n---\nBody";
    let yaml = parse_prompt_file(content, "id", FrontmatterParser::Yaml).unwrap();
    let basic = parse_prompt_file(content, "id", FrontmatterParser::Basic).unwrap();
    assert_eq!(
        serde_json::to_string(&yaml.metadata).unwrap(),
        r#"{"stop":["a","b"]}"#
    );
    assert_eq!(
        serde_json::to_string(&basic.metadata).unwrap(),
        r#"{"stop":"[a, b]"}"#
    );
}

#[rstest]
#[case::lowercase_bool("a: true\nb: false", r#"{"a":true,"b":false}"#)]
#[case::mixed_case_bool("a: True\nb: FALSE", r#"{"a":true,"b":false}"#)]
#[case::integer("a: 42", r#"{"a":42}"#)]
#[case::leading_zeros("a: 007", r#"{"a":7}"#)]
#[case::zero("a: 0", r#"{"a":0}"#)]
#[case::big_integer(
    "a: 123456789012345678901234567890",
    r#"{"a":123456789012345678901234567890}"#
)]
#[case::float("a: 0.5\nb: .5\nc: 3.0", r#"{"a":0.5,"b":0.5,"c":3.0}"#)]
#[case::malformed_float_stays_text("a: 1.2.3\nb: 1..5", r#"{"a":"1.2.3","b":"1..5"}"#)]
#[case::negative_stays_text("a: -1", r#"{"a":"-1"}"#)]
#[case::quotes_are_stripped(
    "a: \"x y\"\nb: 'z'\nc: \"mixed'",
    r#"{"a":"x y","b":"z","c":"mixed"}"#
)]
#[case::only_the_first_colon_splits("url: http://x:80/p", r#"{"url":"http://x:80/p"}"#)]
#[case::empty_value("a:", r#"{"a":""}"#)]
#[case::dot_alone_is_text("a: .", r#"{"a":"."}"#)]
#[case::comments_and_plain_lines_are_skipped("# c: 1\njust text\n  # d: 2\na: 1", r#"{"a":1}"#)]
#[case::later_keys_replace_earlier_ones("a: 1\nb: 2\na: 3", r#"{"a":3,"b":2}"#)]
#[case::keys_and_values_are_trimmed("  k  :   v  ", r#"{"k":"v"}"#)]
#[case::empty_input("", "{}")]
fn the_basic_parser_infers_scalar_types(#[case] yaml: &str, #[case] expected: &str) {
    assert_eq!(
        serde_json::to_string(&parse_yaml_basic(yaml)).unwrap(),
        expected
    );
}
