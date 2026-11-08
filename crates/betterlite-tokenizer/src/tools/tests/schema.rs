use rstest::rstest;

use super::{format, wrapper};

#[rstest]
#[case::descriptions_render_as_comments(
    r#"[{"type":"function","function":{"name":"described","description":"Function comment","parameters":{"type":"object","properties":{"field":{"type":"string","description":"Field comment"}}}}}]"#,
    &[
        "// Function comment",
        "type described = (_: {",
        "// Field comment",
        "field?: string,",
        "}) => any;",
        "",
    ]
)]
#[case::empty_descriptions_render_empty_comments(
    r#"[{"type":"function","function":{"name":"blank","description":"","parameters":{"type":"object","properties":{"field":{"type":"string","description":""}}}}}]"#,
    &["// ", "type blank = (_: {", "// ", "field?: string,", "}) => any;", ""]
)]
#[case::required_markers_follow_the_required_list(
    r#"[{"type":"function","function":{"name":"markers","parameters":{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"string"},"c":{"type":"string"}},"required":["a","c"]}}}]"#,
    &["type markers = (_: {", "a: string,", "b?: string,", "c: string,", "}) => any;", ""]
)]
#[case::string_enums_render_quoted_alternatives(
    r#"[{"type":"function","function":{"name":"colors","parameters":{"type":"object","properties":{"color":{"type":"string","enum":["red","green"]}}}}}]"#,
    &["type colors = (_: {", "color?: \"red\" | \"green\",", "}) => any;", ""]
)]
#[case::integer_enums_render_quoted_numbers(
    r#"[{"type":"function","function":{"name":"retries","parameters":{"type":"object","properties":{"count":{"type":"integer","enum":[1,2,3]}}}}}]"#,
    &["type retries = (_: {", "count?: \"1\" | \"2\" | \"3\",", "}) => any;", ""]
)]
#[case::number_enums_render_quoted_numbers(
    r#"[{"type":"function","function":{"name":"ratios","parameters":{"type":"object","properties":{"ratio":{"type":"number","enum":[1.5,2.5]}}}}}]"#,
    &["type ratios = (_: {", "ratio?: \"1.5\" | \"2.5\",", "}) => any;", ""]
)]
#[case::non_text_enum_values_render_as_json_text(
    r#"[{"type":"function","function":{"name":"flags","parameters":{"type":"object","properties":{"flag":{"type":"string","enum":[true,null]}}}}}]"#,
    &["type flags = (_: {", "flag?: \"true\" | \"null\",", "}) => any;", ""]
)]
#[case::nested_objects_indent_their_properties(
    r#"[{"type":"function","function":{"name":"nested","parameters":{"type":"object","properties":{"inner":{"type":"object","description":"Inner object","properties":{"flag":{"type":"boolean"},"label":{"type":"string"}},"required":["flag"]}}}}}]"#,
    &[
        "type nested = (_: {",
        "// Inner object",
        "inner?: {",
        "  flag: boolean,",
        "  label?: string,",
        "},",
        "}) => any;",
        "",
    ]
)]
#[case::arrays_render_their_item_types(
    r#"[{"type":"function","function":{"name":"lists","parameters":{"type":"object","properties":{"tags":{"type":"array","items":{"type":"string"}},"scores":{"type":"array","items":{"type":"integer"}}}}}}]"#,
    &["type lists = (_: {", "tags?: string[],", "scores?: number[],", "}) => any;", ""]
)]
#[case::arrays_of_objects_indent_the_object(
    r#"[{"type":"function","function":{"name":"groups","parameters":{"type":"object","properties":{"groups":{"type":"array","items":{"type":"object","properties":{"id":{"type":"integer"}}}}}}}}]"#,
    &["type groups = (_: {", "groups?: {", "  id?: number,", "}[],", "}) => any;", ""]
)]
#[case::unknown_and_missing_types_render_any(
    r#"[{"type":"function","function":{"name":"opaque","parameters":{"type":"object","properties":{"unknown":{"type":"opaquetype"},"absent":{}}}}}]"#,
    &["type opaque = (_: {", "unknown?: any,", "absent?: any,", "}) => any;", ""]
)]
#[case::boolean_and_null_types_render_their_names(
    r#"[{"type":"function","function":{"name":"scalars","parameters":{"type":"object","properties":{"flag":{"type":"boolean"},"nothing":{"type":"null"}}}}}]"#,
    &["type scalars = (_: {", "flag?: boolean,", "nothing?: null,", "}) => any;", ""]
)]
#[case::empty_properties_render_no_argument_list(
    r#"[{"type":"function","function":{"name":"empty","parameters":{"type":"object","properties":{}}}}]"#,
    &["type empty = () => any;", ""]
)]
#[case::missing_parameters_render_no_argument_list(
    r#"[{"type":"function","function":{"name":"bare"}}]"#,
    &["type bare = () => any;", ""]
)]
#[case::null_parameters_render_no_argument_list(
    r#"[{"type":"function","function":{"name":"nulled","parameters":null}}]"#,
    &["type nulled = () => any;", ""]
)]
#[case::string_properties_ignore_a_required_flag(
    r#"[{"type":"function","function":{"name":"text","parameters":{"type":"object","properties":{"field":{"type":"string","required":true}}}}}]"#,
    &["type text = (_: {", "field?: string,", "}) => any;", ""]
)]
#[case::object_nodes_ignore_items(
    r#"[{"type":"function","function":{"name":"shaped","parameters":{"type":"object","properties":{"obj":{"type":"object","properties":{"id":{"type":"integer"}},"items":5}}}}}]"#,
    &["type shaped = (_: {", "obj?: {", "  id?: number,", "},", "}) => any;", ""]
)]
#[case::boolean_nodes_ignore_enum(
    r#"[{"type":"function","function":{"name":"flags","parameters":{"type":"object","properties":{"flag":{"type":"boolean","enum":"x"}}}}}]"#,
    &["type flags = (_: {", "flag?: boolean,", "}) => any;", ""]
)]
fn token_formatting_behavior_tests_behavior(#[case] tools: &str, #[case] expected: &[&str]) {
    assert_eq!(format(tools).unwrap(), wrapper(expected));
}
