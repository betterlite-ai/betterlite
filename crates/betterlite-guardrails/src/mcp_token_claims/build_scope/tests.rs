#![cfg(test)]

use rstest::rstest;

use crate::mcp_guardrail_translation::mapping_registration::McpCallType;
use crate::mcp_token_claims::build_scope::generate;

fn scopes(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn configured_scopes_are_joined_verbatim() {
    let configured = scopes(&["mcp:tools/call", "mcp:tools/list"]);
    assert_eq!(
        generate(
            Some(&configured),
            Some("weather"),
            Some(McpCallType::CallMcpTool)
        ),
        "mcp:tools/call mcp:tools/list"
    );
}

#[test]
fn an_explicit_empty_scope_list_yields_an_empty_scope() {
    assert_eq!(
        generate(Some(&[]), Some("weather"), Some(McpCallType::CallMcpTool)),
        ""
    );
}

#[test]
fn a_tool_call_with_a_name_gets_the_call_and_tool_scopes() {
    assert_eq!(
        generate(None, Some("weather"), Some(McpCallType::CallMcpTool)),
        "mcp:tools/call mcp:tools/weather:call"
    );
}

#[rstest]
#[case::dot("weather.v2", "weather_v2")]
#[case::slash("weather/predict", "weather_predict")]
#[case::space("weather predict", "weather_predict")]
#[case::colon("weather:predict", "weather_predict")]
#[case::at_sign("weather@v2", "weather_v2")]
#[case::underscore_kept("weather_v2", "weather_v2")]
#[case::hyphen_kept("weather-v2", "weather-v2")]
#[case::digits_kept("weather2", "weather2")]
#[case::unicode("m\u{e9}t\u{e9}o", "m_t_o")]
#[case::only_invalid("!!!", "___")]
fn tool_names_are_sanitized_to_letters_digits_underscore_and_hyphen(
    #[case] raw: &str,
    #[case] sanitized: &str,
) {
    assert_eq!(
        generate(None, Some(raw), Some(McpCallType::CallMcpTool)),
        format!("mcp:tools/call mcp:tools/{sanitized}:call")
    );
}

#[test]
fn a_tool_call_without_a_name_gets_only_the_call_scope() {
    assert_eq!(
        generate(None, None, Some(McpCallType::CallMcpTool)),
        "mcp:tools/call"
    );
    assert_eq!(
        generate(None, Some(""), Some(McpCallType::CallMcpTool)),
        "mcp:tools/call"
    );
}

#[test]
fn a_list_request_gets_only_the_list_scope() {
    assert_eq!(
        generate(None, None, Some(McpCallType::ListMcpTools)),
        "mcp:tools/list"
    );
}

#[test]
fn a_request_without_a_call_type_is_treated_as_a_list() {
    assert_eq!(generate(None, None, None), "mcp:tools/list");
    assert_eq!(
        generate(None, Some("weather"), None),
        "mcp:tools/call mcp:tools/weather:call"
    );
}

#[test]
fn a_sanitized_name_of_underscores_still_counts_as_a_name() {
    assert_eq!(
        generate(None, Some("!!!"), Some(McpCallType::CallMcpTool)),
        "mcp:tools/call mcp:tools/___:call"
    );
}
