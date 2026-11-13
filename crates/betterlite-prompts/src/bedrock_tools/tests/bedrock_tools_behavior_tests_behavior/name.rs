use crate::bedrock_tools::name::{ToolNameMap, make_valid_bedrock_tool_name};

#[rstest::rstest]
#[case::keeps_a_valid_name("get_weather", "get_weather")]
#[case::keeps_an_upper_case_name("GetWeather", "GetWeather")]
#[case::keeps_underscores_and_hyphens("a_b-c", "a_b-c")]
#[case::keeps_an_ascii_digit_after_the_first_letter("weather2", "weather2")]
#[case::prepends_a_before_a_digit("9lives", "a9lives")]
#[case::prepends_a_before_an_underscore("_private", "a_private")]
#[case::prepends_a_before_punctuation("!!!", "a___")]
#[case::replaces_spaces_and_punctuation("my tool!", "my_tool_")]
#[case::replaces_a_path_separator("a.b/c", "a_b_c")]
#[case::keeps_an_empty_name_empty("", "")]
#[case::makes_a_blank_name_valid("   ", "a___")]
#[case::replaces_non_ascii_letters("tool\u{e9}name", "tool_name")]
#[case::prefixes_a_non_ascii_first_letter("\u{e9}tool", "a_tool")]
fn bedrock_tools_behavior_tests_behavior(#[case] input: &str, #[case] expected: &str) {
    let mut names = ToolNameMap::new();
    let valid = make_valid_bedrock_tool_name(input, &mut names);
    assert_eq!(valid, expected);
    let recorded = names.get(expected).map(String::as_str);
    assert_eq!(recorded, (input != expected).then_some(input));
}
