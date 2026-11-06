use rstest::rstest;

use super::{format, wrapper};

#[rstest]
#[case::openai_function(
    r#"[{"type":"function","function":{"name":"get_weather","description":"Get the current weather","parameters":{"type":"object","properties":{"location":{"type":"string","description":"The city and state"},"unit":{"type":"string","enum":["celsius","fahrenheit"]}},"required":["location"]}}}]"#,
    &[
        "// Get the current weather",
        "type get_weather = (_: {",
        "// The city and state",
        "location: string,",
        "unit?: \"celsius\" | \"fahrenheit\",",
        "}) => any;",
        "",
    ]
)]
#[case::anthropic_input_schema(
    r#"[{"name":"claude_tool","description":"An anthropic tool","input_schema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}}]"#,
    &["// An anthropic tool", "type claude_tool = (_: {", "q: string,", "}) => any;", ""]
)]
#[case::plain_parameters(
    r#"[{"name":"plain","parameters":{"type":"object","properties":{"value":{"type":"integer"}}}}]"#,
    &["type plain = (_: {", "value?: number,", "}) => any;", ""]
)]
#[case::function_takes_precedence(
    r#"[{"type":"function","function":{"name":"first","parameters":{"type":"object","properties":{"a":{"type":"string"}}}},"functionDeclarations":[{"name":"second"}],"input_schema":{"type":"object","properties":{"b":{"type":"string"}}},"name":"third"}]"#,
    &["type first = (_: {", "a?: string,", "}) => any;", ""]
)]
#[case::declarations_take_precedence_over_the_schema(
    r#"[{"functionDeclarations":[{"name":"declared"}],"input_schema":{"type":"object","properties":{"b":{"type":"string"}}},"name":"third"}]"#,
    &["type declared = () => any;", ""]
)]
#[case::explicit_empty_snake_declarations_stay_empty(
    r#"[{"function_declarations":[],"functionDeclarations":[{"name":"camel"}]}]"#,
    &[]
)]
#[case::explicit_empty_input_schema_shadows_parameters(
    r#"[{"input_schema":{},"name":"empty_schema","parameters":{"type":"object","properties":{"a":{"type":"string"}}}}]"#,
    &["type empty_schema = () => any;", ""]
)]
#[case::non_object_tools_are_skipped(
    r#"["text",42,null,[],{"type":"web_search"}]"#,
    &[]
)]
#[case::nameless_function_is_skipped(
    r#"[{"type":"function","function":{"description":"No name"}}]"#,
    &[]
)]
#[case::several_functions_keep_their_order(
    r#"[{"type":"function","function":{"name":"first"}},{"type":"function","function":{"name":"second"}}]"#,
    &["type first = () => any;", "", "type second = () => any;", ""]
)]
#[case::property_order_is_preserved(
    r#"[{"type":"function","function":{"name":"ordered","parameters":{"type":"object","properties":{"zebra":{"type":"string"},"alpha":{"type":"string"}}}}}]"#,
    &["type ordered = (_: {", "zebra?: string,", "alpha?: string,", "}) => any;", ""]
)]
fn token_formatting_behavior_tests_behavior(#[case] tools: &str, #[case] expected: &[&str]) {
    assert_eq!(format(tools).unwrap(), wrapper(expected));
}
