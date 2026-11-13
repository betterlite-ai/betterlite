use betterlite_prompts::arguments::json::Json;
use serde::Serialize;

use crate::tools::wire::{ChoiceKind, McpServerTool, ToolDefinition};
use crate::tools::{
    ToolMapping, ToolsError, map_openai_mcp_server_tool, map_tool_choice, map_tool_helper,
    map_tools,
};

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn as_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    let text = serde_json::to_string(value).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn mapping(source: &str) -> ToolMapping {
    map_tools(&[json(source)]).unwrap()
}

fn assert_tools(source: &str, expected: &str) {
    assert_eq!(as_json(&mapping(source).tools), json(expected));
}

fn assert_servers(source: &str, expected: &str) {
    assert_eq!(as_json(&mapping(source).mcp_servers), json(expected));
}

fn helper_failure(source: &str) -> ToolsError {
    match map_tool_helper(&json(source)) {
        Ok(_) => panic!("expected a helper failure"),
        Err(error) => error,
    }
}

fn tools_failure(source: &str) -> ToolsError {
    match map_tools(&[json(source)]) {
        Ok(_) => panic!("expected a tools failure"),
        Err(error) => error,
    }
}

fn assert_helper_error(source: &str, expected: &str) {
    assert_eq!(helper_failure(source).to_string(), expected);
}

fn assert_tools_error(source: &str, expected: &str) {
    assert_eq!(tools_failure(source).to_string(), expected);
}

fn assert_choice(source: &str, parallel: Option<bool>, expected: &str) {
    let choice = map_tool_choice(Some(&json(source)), parallel).unwrap();
    assert_eq!(choice.map(|mapped| as_json(&mapped)), Some(json(expected)));
}

fn assert_no_choice(source: &str, parallel: Option<bool>) {
    assert!(
        map_tool_choice(Some(&json(source)), parallel)
            .unwrap()
            .is_none()
    );
}

fn assert_choice_error(source: &str, parallel: Option<bool>, expected: &str) {
    let failure = map_tool_choice(Some(&json(source)), parallel).unwrap_err();
    assert_eq!(failure.to_string(), expected);
}

#[test]
fn case_1_function_input_schema() {
    function_schema_shapes();
    function_schema_eager();
    function_schema_failures();
}

fn function_schema_shapes() {
    assert_tools(
        r#"{"type":"function","function":{"name":"weather","parameters":{"type":"object","properties":{"city":{"type":"string"}}},"description":"look up"}}"#,
        r#"[{"name":"weather","input_schema":{"type":"object","properties":{"city":{"type":"string"}}},"type":"custom","description":"look up"}]"#,
    );
    assert_tools(
        r#"{"type":"function","function":{"name":"bare"}}"#,
        r#"[{"name":"bare","input_schema":{"type":"object","properties":{}},"type":"custom"}]"#,
    );
    assert_tools(
        r#"{"type":"custom","function":{"name":"empty","description":"","parameters":{"type":"object","strict":true,"unknown":1}}}"#,
        r#"[{"name":"empty","input_schema":{"type":"object","properties":{},"strict":true},"type":"custom","description":""}]"#,
    );
}

fn function_schema_eager() {
    assert_tools(
        r#"{"type":"function","function":{"name":"eager","eager_input_streaming":false,"parameters":{"properties":{"x":{"type":"integer"}}}}}"#,
        r#"[{"name":"eager","input_schema":{"type":"object","properties":{"x":{"type":"integer"}}},"type":"custom","eager_input_streaming":false}]"#,
    );
    assert_tools(
        r#"{"type":"function","function":{"name":"top-eager"},"eager_input_streaming":true}"#,
        r#"[{"name":"top-eager","input_schema":{"type":"object","properties":{}},"type":"custom","eager_input_streaming":true}]"#,
    );
}

fn function_schema_failures() {
    assert_helper_error(
        r#"{"type":"function","function":{"parameters":{}}}"#,
        "Missing required parameter: name",
    );
    assert_helper_error(
        r#"{"type":"function","function":{"name":"x","eager_input_streaming":"yes"}}"#,
        "eager_input_streaming must be a boolean",
    );
    let error =
        helper_failure(r#"{"type":"function","function":{"name":"x","eager_input_streaming":1}}"#);
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn case_2_computer_hosted_tools() {
    computer_shapes();
    hosted_shapes();
    computer_failures();
}

fn computer_shapes() {
    assert_tools(
        r#"{"type":"computer_20241022","function":{"parameters":{"display_width_px":0,"display_height_px":0,"display_number":2}}}"#,
        r#"[{"type":"computer_20241022","name":"computer","display_width_px":0,"display_height_px":0,"display_number":2}]"#,
    );
    assert_tools(
        r#"{"type":"computer_20250124","function":{"name":"","parameters":{"display_width_px":1,"display_height_px":2,"display_number":null}}}"#,
        r#"[{"type":"computer_20250124","name":"","display_width_px":1,"display_height_px":2}]"#,
    );
}

fn hosted_shapes() {
    assert_tools(
        r#"{"type":"web_search_20250305","name":"search","max_uses":5}"#,
        r#"[{"type":"web_search_20250305","name":"search","max_uses":5}]"#,
    );
    assert_tools(
        r#"{"type":"bash_20250124","function":{"name":"shell","description":"run"}}"#,
        r#"[{"type":"bash_20250124","name":"shell","function":{"name":"shell","description":"run"}}]"#,
    );
    assert_tools(
        r#"{"type":"text_editor_20250124","name":"editor","function":{"name":"other"}}"#,
        r#"[{"type":"text_editor_20250124","name":"editor","function":{"name":"other"}}]"#,
    );
    assert_tools(
        r#"{"type":"code_execution_20250825","name":""}"#,
        r#"[{"type":"code_execution_20250825","name":""}]"#,
    );
}

fn computer_failures() {
    assert_helper_error(
        r#"{"type":"computer_20250124","function":{}}"#,
        "Missing required parameter: parameters",
    );
    assert_helper_error(
        r#"{"type":"computer_20250124","function":{"parameters":{"display_width_px":80}}}"#,
        "Missing required parameter: display_width_px or display_height_px",
    );
    assert_helper_error(
        r#"{"type":"web_search_20250305","maximum_uses":1}"#,
        "Missing required parameter: name",
    );
    assert_helper_error(
        r#"{"type":"web_fetch_20250910","name":7}"#,
        "Missing required parameter: name",
    );
}

#[test]
fn case_3_tool_choice_any_none_auto() {
    choice_strings();
    choice_objects();
    choice_named_tools();
    choice_failures();
}

fn choice_strings() {
    assert_choice(r#""auto""#, None, r#"{"type":"auto"}"#);
    assert_choice(r#""required""#, None, r#"{"type":"any"}"#);
    assert_choice(r#""none""#, None, r#"{"type":"none"}"#);
    assert_no_choice(r#""bogus""#, None);
    assert_no_choice("null", None);
    assert_eq!(
        map_tool_choice(Some(&json(r#""required""#)), None)
            .unwrap()
            .unwrap()
            .kind,
        ChoiceKind::Any
    );
}

fn choice_objects() {
    assert_choice(r#"{"type":"auto"}"#, None, r#"{"type":"auto"}"#);
    assert_choice(r#"{"type":"required"}"#, None, r#"{"type":"any"}"#);
    assert_choice(r#"{"type":"any"}"#, None, r#"{"type":"any"}"#);
    assert_choice(r#"{"type":"none"}"#, None, r#"{"type":"none"}"#);
    assert_no_choice(r#"{"type":"tool","name":"lookup"}"#, None);
    assert_no_choice(r#"{"type":"unknown","name":"lookup"}"#, None);
}

fn choice_named_tools() {
    assert_choice(
        r#"{"function":{"name":"lookup"}}"#,
        None,
        r#"{"type":"tool","name":"lookup"}"#,
    );
    assert_choice(
        r#"{"function":{"name":""}}"#,
        None,
        r#"{"type":"tool","name":""}"#,
    );
    assert_choice(
        r#"{"type":"function","function":{"name":"lookup"}}"#,
        None,
        r#"{"type":"tool","name":"lookup"}"#,
    );
    assert_no_choice(r#"{"function":{}}"#, None);
    assert_no_choice(r#"{"function":{"name":null}}"#, None);
}

fn choice_failures() {
    assert_choice_error("7", None, "tool_choice must be a string or an object");
    assert_choice_error("[]", None, "tool_choice must be a string or an object");
    assert_choice_error(
        r#"{"function":"named"}"#,
        None,
        "tool_choice.function must be an object",
    );
    assert_choice_error(
        r#"{"function":{"name":7}}"#,
        None,
        "tool_choice.function.name must be a string",
    );
}

#[test]
fn case_4_parallel_tool_use() {
    parallel_defaults();
    parallel_none_suppression();
}

fn parallel_defaults() {
    assert_choice(
        "null",
        Some(true),
        r#"{"type":"auto","disable_parallel_tool_use":false}"#,
    );
    assert_choice(
        "null",
        Some(false),
        r#"{"type":"auto","disable_parallel_tool_use":true}"#,
    );
    assert_choice(
        r#""auto""#,
        Some(false),
        r#"{"type":"auto","disable_parallel_tool_use":true}"#,
    );
    assert_choice(
        r#""required""#,
        Some(true),
        r#"{"type":"any","disable_parallel_tool_use":false}"#,
    );
    assert_choice(
        r#"{"function":{"name":"lookup"}}"#,
        Some(true),
        r#"{"type":"tool","name":"lookup","disable_parallel_tool_use":false}"#,
    );
    assert_choice(
        r#"{"function":{}}"#,
        Some(true),
        r#"{"type":"auto","disable_parallel_tool_use":false}"#,
    );
}

fn parallel_none_suppression() {
    assert_choice(r#""none""#, Some(true), r#"{"type":"none"}"#);
    assert_choice(r#""none""#, Some(false), r#"{"type":"none"}"#);
    assert_choice(
        r#"{"type":"none"}"#,
        Some(true),
        r#"{"type":"none","disable_parallel_tool_use":false}"#,
    );
    assert_choice(r#""auto""#, None, r#"{"type":"auto"}"#);
}

#[test]
fn case_5_mcp_server_configuration() {
    mcp_descriptors();
    mcp_mapping();
    mcp_failures();
}

fn mcp_descriptors() {
    assert_servers(
        r#"{"type":"url","url":"https://mcp.test/rpc","name":"remote","headers":{"Authorization":"Bearer raw"},"require_approval":"never"}"#,
        r#"[{"type":"url","url":"https://mcp.test/rpc","name":"remote","headers":{"Authorization":"Bearer raw"},"require_approval":"never"}]"#,
    );
}

fn mcp_mapping() {
    assert_servers(
        r#"{"type":"mcp","server_url":"https://mcp.test/rpc","server_label":"label","allowed_tools":[],"headers":{"Authorization":"Bearer x Bearer y","X-Other":"keep"},"require_approval":"never"}"#,
        r#"[{"type":"url","url":"https://mcp.test/rpc","name":"label","tool_configuration":{"allowed_tools":[]},"authorization_token":"x y"}]"#,
    );
    assert_servers(
        r#"{"type":"mcp","server_url":"https://mcp.test/rpc","server_label":"label","headers":{"Authorization":"Bearer "}}"#,
        r#"[{"type":"url","url":"https://mcp.test/rpc","name":"label","authorization_token":""}]"#,
    );
    assert_servers(
        r#"{"type":"mcp","server_url":"https://mcp.test/rpc","server_label":"label","headers":{"authorization":"Bearer ignored"},"allowed_tools":["one"]}"#,
        r#"[{"type":"url","url":"https://mcp.test/rpc","name":"label","tool_configuration":{"allowed_tools":["one"]}}]"#,
    );
    assert_servers(
        r#"{"type":"mcp","server_url":"https://mcp.test/rpc","server_label":"label","headers":null,"allowed_tools":null}"#,
        r#"[{"type":"url","url":"https://mcp.test/rpc","name":"label"}]"#,
    );
}

fn mcp_failures() {
    assert_helper_error(
        r#"{"type":"mcp","server_label":"label"}"#,
        "Missing required parameter: server_url",
    );
    assert_helper_error(
        r#"{"type":"mcp","server_url":"https://mcp.test/rpc"}"#,
        "Missing required parameter: server_label",
    );
    assert_helper_error(
        r#"{"type":"mcp","server_url":"u","server_label":"l","allowed_tools":"one"}"#,
        "allowed_tools must be a list of strings",
    );
    assert_helper_error(
        r#"{"type":"mcp","server_url":"u","server_label":"l","headers":{"Authorization":7}}"#,
        "headers.Authorization must be a string",
    );
    map_openai_mcp_server_tool(&json(r#"{"server_url":"u","server_label":"l"}"#)).unwrap();
}

#[test]
fn case_6_advisor_tool() {
    assert_tools(
        r#"{"type":"advisor_20260301","model":"claude-opus-5-5","max_uses":3,"caching":{"type":"ephemeral"}}"#,
        r#"[{"type":"advisor_20260301","name":"advisor","model":"claude-opus-5-5","max_uses":3,"caching":{"type":"ephemeral"}}]"#,
    );
    assert_tools(
        r#"{"type":"advisor_20260301","model":"","max_uses":null,"caching":null}"#,
        r#"[{"type":"advisor_20260301","name":"advisor","model":""}]"#,
    );
    assert_helper_error(
        r#"{"type":"advisor_20260301"}"#,
        "Advisor tool must have a valid model",
    );
    assert_helper_error(
        r#"{"type":"advisor_20260301","model":7}"#,
        "Advisor tool must have a valid model",
    );
    assert_helper_error(
        r#"{"type":"unknown_20260101","model":"m"}"#,
        "Unsupported tool type: unknown_20260101",
    );
}

#[test]
fn case_7_cache_examples_callers() {
    cache_precedence();
    caller_precedence();
    example_precedence();
    option_failures();
}

fn cache_precedence() {
    assert_tools(
        r#"{"type":"function","function":{"name":"cached","cache_control":{"ttl":"1h"}},"cache_control":{"type":"ephemeral"}}"#,
        r#"[{"name":"cached","input_schema":{"type":"object","properties":{}},"type":"custom","cache_control":{"type":"ephemeral"}}]"#,
    );
    assert_tools(
        r#"{"type":"function","function":{"name":"nested","cache_control":{"type":"ephemeral"}}}"#,
        r#"[{"name":"nested","input_schema":{"type":"object","properties":{}},"type":"custom","cache_control":{"type":"ephemeral"}}]"#,
    );
    assert_tools(
        r#"{"type":"web_search_20250305","name":"s","function":{"cache_control":{"type":"ephemeral"}}}"#,
        r#"[{"type":"web_search_20250305","name":"s","function":{"cache_control":{"type":"ephemeral"}},"cache_control":{"type":"ephemeral"}}]"#,
    );
}

fn caller_precedence() {
    assert_tools(
        r#"{"type":"function","function":{"name":"callers","allowed_callers":["code_execution_20250825"]},"allowed_callers":["direct"]}"#,
        r#"[{"name":"callers","input_schema":{"type":"object","properties":{}},"type":"custom","allowed_callers":["direct"]}]"#,
    );
    assert_tools(
        r#"{"type":"function","function":{"name":"nested-callers","allowed_callers":[]}}"#,
        r#"[{"name":"nested-callers","input_schema":{"type":"object","properties":{}},"type":"custom","allowed_callers":[]}]"#,
    );
}

fn example_precedence() {
    assert_tools(
        r#"{"type":"function","function":{"name":"fallback","input_examples":[{"q":1}]},"input_examples":{"bad":1}}"#,
        r#"[{"name":"fallback","input_schema":{"type":"object","properties":{}},"type":"custom","input_examples":[{"q":1}]}]"#,
    );
    assert_tools(
        r#"{"type":"function","function":{"name":"top-examples","input_examples":[{"q":1}]},"input_examples":[{"q":2}]}"#,
        r#"[{"name":"top-examples","input_schema":{"type":"object","properties":{}},"type":"custom","input_examples":[{"q":2}]}]"#,
    );
    assert_tools(
        r#"{"type":"function","function":{"name":"ignored"},"input_examples":{"bad":1}}"#,
        r#"[{"name":"ignored","input_schema":{"type":"object","properties":{}},"type":"custom"}]"#,
    );
}

fn option_failures() {
    assert_helper_error(
        r#"{"type":"function","function":{"name":"x","allowed_callers":[1]}}"#,
        "allowed_callers must be a list of strings",
    );
    assert_helper_error(
        r#"{"type":"function","function":{"name":"x"},"allowed_callers":"direct"}"#,
        "allowed_callers must be a list of strings",
    );
}

#[test]
fn case_8_eager_input_streaming_defer_loading() {
    defer_precedence();
    defer_suppression();
    defer_failures();
}

fn defer_precedence() {
    assert_tools(
        r#"{"type":"function","function":{"name":"deferred","defer_loading":false}}"#,
        r#"[{"name":"deferred","input_schema":{"type":"object","properties":{}},"type":"custom","defer_loading":false}]"#,
    );
    assert_tools(
        r#"{"type":"function","function":{"name":"top-defer"},"defer_loading":true}"#,
        r#"[{"name":"top-defer","input_schema":{"type":"object","properties":{}},"type":"custom","defer_loading":true}]"#,
    );
    assert_tools(
        r#"{"type":"function","function":{"name":"nested-defer","defer_loading":null},"defer_loading":null}"#,
        r#"[{"name":"nested-defer","input_schema":{"type":"object","properties":{}},"type":"custom"}]"#,
    );
}

fn defer_suppression() {
    assert_tools(
        r#"{"type":"computer_20241022","function":{"parameters":{"display_width_px":1,"display_height_px":2},"defer_loading":true,"allowed_callers":["direct"]}}"#,
        r#"[{"type":"computer_20241022","name":"computer","display_width_px":1,"display_height_px":2}]"#,
    );
    assert_tools(
        r#"{"type":"computer_20241023","function":{"parameters":{"display_width_px":1,"display_height_px":2},"defer_loading":true}}"#,
        r#"[{"type":"computer_20241023","name":"computer","display_width_px":1,"display_height_px":2,"defer_loading":true}]"#,
    );
    assert_tools(
        r#"{"type":"web_search_20250305","name":"s","defer_loading":true,"allowed_callers":["direct"]}"#,
        r#"[{"type":"web_search_20250305","name":"s","defer_loading":true,"allowed_callers":["direct"]}]"#,
    );
    assert_tools(
        r#"{"type":"bash_20250124","name":"shell","eager_input_streaming":"yes"}"#,
        r#"[{"type":"bash_20250124","name":"shell","eager_input_streaming":"yes"}]"#,
    );
}

fn defer_failures() {
    assert_helper_error(
        r#"{"type":"function","function":{"name":"x","defer_loading":"yes"}}"#,
        "defer_loading must be a boolean",
    );
    assert_helper_error(
        r#"{"type":"function","function":{"name":"x"},"defer_loading":1}"#,
        "defer_loading must be a boolean",
    );
}

#[test]
fn case_9_namespace_and_native_tools() {
    namespace_expansion();
    native_passthrough();
    namespace_failures();
}

fn namespace_expansion() {
    assert_tools(
        r#"{"type":"namespace","tools":[{"input_schema":{"type":"object","x":1},"name":"native","type":"custom","cache_control":{"type":"ephemeral"}},{"name":"flat","description":"d","parameters":{"properties":{"a":{"type":"string"}}},"cache_control":{"type":"ephemeral"}},{"type":"function","function":{"name":"nested","cache_control":{"type":"ephemeral"}}},{"type":"web_search_20250305"},{"description":"skipped"}]}"#,
        r#"[{"input_schema":{"type":"object","x":1},"name":"native","type":"custom","cache_control":{"type":"ephemeral"}},{"name":"flat","input_schema":{"type":"object","properties":{"a":{"type":"string"}}},"type":"custom","description":"d","cache_control":{"type":"ephemeral"}},{"name":"nested","input_schema":{"type":"object","properties":{}},"type":"custom","cache_control":{"type":"ephemeral"}}]"#,
    );
    // a nested MCP descriptor has no name and no function, so the namespace loop skips it
    let skipped = mapping(
        r#"{"type":"namespace","tools":[{"name":"flat"},{"type":"mcp","server_url":"u","server_label":"l"}]}"#,
    );
    assert_eq!(skipped.tools.len(), 1);
    assert_eq!(skipped.mcp_servers.len(), 0);
    let ordered = map_tools(&[
        json(r#"{"type":"mcp","server_url":"u","server_label":"first"}"#),
        json(r#"{"type":"function","function":{"name":"one"}}"#),
        json(r#"{"type":"mcp","server_url":"u","server_label":"second"}"#),
        json(r#"{"type":"function","function":{"name":"two"}}"#),
    ])
    .unwrap();
    let names: Vec<String> = ordered
        .tools
        .iter()
        .filter_map(|tool| match tool {
            ToolDefinition::Function(function) => Some(function.name.clone()),
            ToolDefinition::Computer(computer) => Some(computer.name.clone()),
            ToolDefinition::Hosted(hosted) => Some(hosted.name.clone()),
            ToolDefinition::Advisor(_) | ToolDefinition::Native(_) => None,
        })
        .collect();
    assert_eq!(names, ["one", "two"]);
    let labels: Vec<String> = ordered
        .mcp_servers
        .iter()
        .filter_map(|server| match server {
            McpServerTool::Mapped(mapped) => Some(mapped.name.clone()),
            McpServerTool::Descriptor(fields) => {
                fields.get("name").and_then(Json::as_str).map(str::to_owned)
            }
        })
        .collect();
    assert_eq!(labels, ["first", "second"]);
}

fn native_passthrough() {
    assert_tools(
        r#"{"type":"custom","name":"kept","input_schema":{"type":"object","unknown":true},"defer_loading":true}"#,
        r#"[{"type":"custom","name":"kept","input_schema":{"type":"object","unknown":true},"defer_loading":true}]"#,
    );
    assert_tools(
        r#"{"type":"custom","name":"first","input_schema":{}}"#,
        r#"[{"type":"custom","name":"first","input_schema":{}}]"#,
    );
}

fn namespace_failures() {
    assert_tools_error(
        r#"{"description":"nameless"}"#,
        "Missing required parameter: type",
    );
    assert_tools_error(
        r#"{"type":"namespace","tools":"bad"}"#,
        "tools must be a list",
    );
    assert_tools_error("7", "tool must be a JSON object");
}

#[test]
fn case_10_search_tool_branch_shadowing() {
    search_types_are_hosted();
    search_suppresses_nested_options();
    search_requires_a_name();
}

fn search_types_are_hosted() {
    assert_tools(
        r#"{"type":"tool_search_tool_regex_20251119","name":"search","max_uses":2,"cache_control":{"type":"ephemeral"},"defer_loading":true,"allowed_callers":["direct"],"input_examples":[{"q":1}]}"#,
        r#"[{"type":"tool_search_tool_regex_20251119","name":"search","max_uses":2,"cache_control":{"type":"ephemeral"},"defer_loading":true,"allowed_callers":["direct"],"input_examples":[{"q":1}]}]"#,
    );
    assert_tools(
        r#"{"type":"tool_search_tool_bm25_20251119","name":""}"#,
        r#"[{"type":"tool_search_tool_bm25_20251119","name":""}]"#,
    );
}

fn search_suppresses_nested_options() {
    assert_tools(
        r#"{"type":"tool_search_tool_regex_20251119","name":"search","function":{"cache_control":{"type":"ephemeral"},"defer_loading":true,"allowed_callers":["direct"]}}"#,
        r#"[{"type":"tool_search_tool_regex_20251119","name":"search","function":{"cache_control":{"type":"ephemeral"},"defer_loading":true,"allowed_callers":["direct"]}}]"#,
    );
}

fn search_requires_a_name() {
    assert_helper_error(
        r#"{"type":"tool_search_tool_regex_20251119","function":{}}"#,
        "Missing required parameter: name",
    );
    assert_helper_error(
        r#"{"type":"tool_search_tool_bm25_20251119"}"#,
        "Missing required parameter: name",
    );
    let error = tools_failure(r#"{"type":"tool_search_tool_bm25_20251119"}"#);
    assert!(matches!(error, ToolsError::Missing("name")));
}
