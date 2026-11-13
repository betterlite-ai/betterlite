use betterlite_prompts::arguments::json::Json;
use serde::Serialize;

use crate::tools::wire::{ChoiceKind, McpServerTool, ToolDefinition};
use crate::tools::{
    ToolMapping, ToolsError, map_openai_mcp_server_tool, map_tool_choice, map_tool_helper,
    map_tools,
};

type Pair = (&'static str, &'static str);
type ChoiceCase = (&'static str, Option<bool>, &'static str);

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
    let choice = map_tool_choice(Some(&json(source)), parallel).unwrap();
    assert!(choice.is_none());
}

fn assert_choice_error(source: &str, parallel: Option<bool>, expected: &str) {
    let failure = map_tool_choice(Some(&json(source)), parallel).unwrap_err();
    assert_eq!(failure.to_string(), expected);
}

fn check_tools(pairs: &[Pair]) {
    for (source, expected) in pairs {
        assert_tools(source, expected);
    }
}

fn check_servers(pairs: &[Pair]) {
    for (source, expected) in pairs {
        assert_servers(source, expected);
    }
}

fn check_errors(pairs: &[Pair]) {
    for (source, expected) in pairs {
        assert_helper_error(source, expected);
    }
}

fn check_tools_errors(pairs: &[Pair]) {
    for (source, expected) in pairs {
        assert_tools_error(source, expected);
    }
}

fn check_choices(pairs: &[ChoiceCase]) {
    for (source, parallel, expected) in pairs {
        assert_choice(source, *parallel, expected);
    }
}

fn check_no_choices(pairs: &[(&str, Option<bool>)]) {
    for (source, parallel) in pairs {
        assert_no_choice(source, *parallel);
    }
}

fn check_choice_errors(pairs: &[ChoiceCase]) {
    for (source, parallel, expected) in pairs {
        assert_choice_error(source, *parallel, expected);
    }
}

const FUNCTION_SCHEMA_SHAPES: [Pair; 3] = [
    (
        r#"{"type":"function","function":{"name":"weather","parameters":{"type":"object","properties":{"city":{"type":"string"}}},"description":"look up"}}"#,
        r#"[{"name":"weather","input_schema":{"type":"object","properties":{"city":{"type":"string"}}},"type":"custom","description":"look up"}]"#,
    ),
    (
        r#"{"type":"function","function":{"name":"bare"}}"#,
        r#"[{"name":"bare","input_schema":{"type":"object","properties":{}},"type":"custom"}]"#,
    ),
    (
        r#"{"type":"custom","function":{"name":"empty","description":"","parameters":{"type":"object","strict":true,"unknown":1}}}"#,
        r#"[{"name":"empty","input_schema":{"type":"object","properties":{},"strict":true},"type":"custom","description":""}]"#,
    ),
];

const FUNCTION_SCHEMA_EAGER: [Pair; 2] = [
    (
        r#"{"type":"function","function":{"name":"eager","eager_input_streaming":false,"parameters":{"properties":{"x":{"type":"integer"}}}}}"#,
        r#"[{"name":"eager","input_schema":{"type":"object","properties":{"x":{"type":"integer"}}},"type":"custom","eager_input_streaming":false}]"#,
    ),
    (
        r#"{"type":"function","function":{"name":"top-eager"},"eager_input_streaming":true}"#,
        r#"[{"name":"top-eager","input_schema":{"type":"object","properties":{}},"type":"custom","eager_input_streaming":true}]"#,
    ),
];

const FUNCTION_SCHEMA_FAILURES: [Pair; 2] = [
    (
        r#"{"type":"function","function":{"parameters":{}}}"#,
        "Missing required parameter: name",
    ),
    (
        r#"{"type":"function","function":{"name":"x","eager_input_streaming":"yes"}}"#,
        "eager_input_streaming must be a boolean",
    ),
];

#[test]
fn case_1_function_input_schema() {
    check_tools(&FUNCTION_SCHEMA_SHAPES);
    check_tools(&FUNCTION_SCHEMA_EAGER);
    check_errors(&FUNCTION_SCHEMA_FAILURES);
    let eager =
        helper_failure(r#"{"type":"function","function":{"name":"x","eager_input_streaming":1}}"#);
    assert_eq!(eager.status_code(), Some(400));
}

const COMPUTER_SHAPES: [Pair; 2] = [
    (
        r#"{"type":"computer_20241022","function":{"parameters":{"display_width_px":0,"display_height_px":0,"display_number":2}}}"#,
        r#"[{"type":"computer_20241022","name":"computer","display_width_px":0,"display_height_px":0,"display_number":2}]"#,
    ),
    (
        r#"{"type":"computer_20250124","function":{"name":"","parameters":{"display_width_px":1,"display_height_px":2,"display_number":null}}}"#,
        r#"[{"type":"computer_20250124","name":"","display_width_px":1,"display_height_px":2}]"#,
    ),
];

const HOSTED_SHAPES: [Pair; 4] = [
    (
        r#"{"type":"web_search_20250305","name":"search","max_uses":5}"#,
        r#"[{"type":"web_search_20250305","name":"search","max_uses":5}]"#,
    ),
    (
        r#"{"type":"bash_20250124","function":{"name":"shell","description":"run"}}"#,
        r#"[{"type":"bash_20250124","name":"shell","function":{"name":"shell","description":"run"}}]"#,
    ),
    (
        r#"{"type":"text_editor_20250124","name":"editor","function":{"name":"other"}}"#,
        r#"[{"type":"text_editor_20250124","name":"editor","function":{"name":"other"}}]"#,
    ),
    (
        r#"{"type":"code_execution_20250825","name":""}"#,
        r#"[{"type":"code_execution_20250825","name":""}]"#,
    ),
];

const COMPUTER_FAILURES: [Pair; 4] = [
    (
        r#"{"type":"computer_20250124","function":{}}"#,
        "Missing required parameter: parameters",
    ),
    (
        r#"{"type":"computer_20250124","function":{"parameters":{"display_width_px":80}}}"#,
        "Missing required parameter: display_width_px or display_height_px",
    ),
    (
        r#"{"type":"web_search_20250305","maximum_uses":1}"#,
        "Missing required parameter: name",
    ),
    (
        r#"{"type":"web_fetch_20250910","name":7}"#,
        "Missing required parameter: name",
    ),
];

#[test]
fn case_2_computer_hosted_tools() {
    check_tools(&COMPUTER_SHAPES);
    check_tools(&HOSTED_SHAPES);
    check_errors(&COMPUTER_FAILURES);
}

const CHOICE_STRINGS: [ChoiceCase; 3] = [
    (r#""auto""#, None, r#"{"type":"auto"}"#),
    (r#""required""#, None, r#"{"type":"any"}"#),
    (r#""none""#, None, r#"{"type":"none"}"#),
];

const CHOICE_STRING_NONE: [(&str, Option<bool>); 2] = [(r#""bogus""#, None), ("null", None)];

const CHOICE_OBJECTS: [ChoiceCase; 4] = [
    (r#"{"type":"auto"}"#, None, r#"{"type":"auto"}"#),
    (r#"{"type":"required"}"#, None, r#"{"type":"any"}"#),
    (r#"{"type":"any"}"#, None, r#"{"type":"any"}"#),
    (r#"{"type":"none"}"#, None, r#"{"type":"none"}"#),
];

const CHOICE_OBJECT_NONE: [(&str, Option<bool>); 2] = [
    (r#"{"type":"tool","name":"lookup"}"#, None),
    (r#"{"type":"unknown","name":"lookup"}"#, None),
];

const CHOICE_NAMED: [ChoiceCase; 3] = [
    (
        r#"{"function":{"name":"lookup"}}"#,
        None,
        r#"{"type":"tool","name":"lookup"}"#,
    ),
    (
        r#"{"function":{"name":""}}"#,
        None,
        r#"{"type":"tool","name":""}"#,
    ),
    (
        r#"{"type":"function","function":{"name":"lookup"}}"#,
        None,
        r#"{"type":"tool","name":"lookup"}"#,
    ),
];

const CHOICE_NAMED_NONE: [(&str, Option<bool>); 2] = [
    (r#"{"function":{}}"#, None),
    (r#"{"function":{"name":null}}"#, None),
];

const CHOICE_FAILURES: [ChoiceCase; 4] = [
    ("7", None, "tool_choice must be a string or an object"),
    ("[]", None, "tool_choice must be a string or an object"),
    (
        r#"{"function":"named"}"#,
        None,
        "tool_choice.function must be an object",
    ),
    (
        r#"{"function":{"name":7}}"#,
        None,
        "tool_choice.function.name must be a string",
    ),
];

#[test]
fn case_3_tool_choice_any_none_auto() {
    check_choices(&CHOICE_STRINGS);
    check_no_choices(&CHOICE_STRING_NONE);
    check_choices(&CHOICE_OBJECTS);
    check_no_choices(&CHOICE_OBJECT_NONE);
    check_choices(&CHOICE_NAMED);
    check_no_choices(&CHOICE_NAMED_NONE);
    check_choice_errors(&CHOICE_FAILURES);
    assert_eq!(
        map_tool_choice(Some(&json(r#""required""#)), None)
            .unwrap()
            .unwrap()
            .kind,
        ChoiceKind::Any
    );
}

const PARALLEL_DEFAULTS: [ChoiceCase; 6] = [
    (
        "null",
        Some(true),
        r#"{"type":"auto","disable_parallel_tool_use":false}"#,
    ),
    (
        "null",
        Some(false),
        r#"{"type":"auto","disable_parallel_tool_use":true}"#,
    ),
    (
        r#""auto""#,
        Some(false),
        r#"{"type":"auto","disable_parallel_tool_use":true}"#,
    ),
    (
        r#""required""#,
        Some(true),
        r#"{"type":"any","disable_parallel_tool_use":false}"#,
    ),
    (
        r#"{"function":{"name":"lookup"}}"#,
        Some(true),
        r#"{"type":"tool","name":"lookup","disable_parallel_tool_use":false}"#,
    ),
    (
        r#"{"function":{}}"#,
        Some(true),
        r#"{"type":"auto","disable_parallel_tool_use":false}"#,
    ),
];

const PARALLEL_SUPPRESSION: [ChoiceCase; 4] = [
    (r#""none""#, Some(true), r#"{"type":"none"}"#),
    (r#""none""#, Some(false), r#"{"type":"none"}"#),
    (
        r#"{"type":"none"}"#,
        Some(true),
        r#"{"type":"none","disable_parallel_tool_use":false}"#,
    ),
    (r#""auto""#, None, r#"{"type":"auto"}"#),
];

#[test]
fn case_4_parallel_tool_use() {
    check_choices(&PARALLEL_DEFAULTS);
    check_choices(&PARALLEL_SUPPRESSION);
}

const MCP_DESCRIPTORS: [Pair; 1] = [(
    r#"{"type":"url","url":"https://mcp.test/rpc","name":"remote","headers":{"Authorization":"Bearer raw"},"require_approval":"never"}"#,
    r#"[{"type":"url","url":"https://mcp.test/rpc","name":"remote","headers":{"Authorization":"Bearer raw"},"require_approval":"never"}]"#,
)];

const MCP_MAPPING: [Pair; 4] = [
    (
        r#"{"type":"mcp","server_url":"https://mcp.test/rpc","server_label":"label","allowed_tools":[],"headers":{"Authorization":"Bearer x Bearer y","X-Other":"keep"},"require_approval":"never"}"#,
        r#"[{"type":"url","url":"https://mcp.test/rpc","name":"label","tool_configuration":{"allowed_tools":[]},"authorization_token":"x y"}]"#,
    ),
    (
        r#"{"type":"mcp","server_url":"https://mcp.test/rpc","server_label":"label","headers":{"Authorization":"Bearer "}}"#,
        r#"[{"type":"url","url":"https://mcp.test/rpc","name":"label","authorization_token":""}]"#,
    ),
    (
        r#"{"type":"mcp","server_url":"https://mcp.test/rpc","server_label":"label","headers":{"authorization":"Bearer ignored"},"allowed_tools":["one"]}"#,
        r#"[{"type":"url","url":"https://mcp.test/rpc","name":"label","tool_configuration":{"allowed_tools":["one"]}}]"#,
    ),
    (
        r#"{"type":"mcp","server_url":"https://mcp.test/rpc","server_label":"label","headers":null,"allowed_tools":null}"#,
        r#"[{"type":"url","url":"https://mcp.test/rpc","name":"label"}]"#,
    ),
];

const MCP_FAILURES: [Pair; 4] = [
    (
        r#"{"type":"mcp","server_label":"label"}"#,
        "Missing required parameter: server_url",
    ),
    (
        r#"{"type":"mcp","server_url":"https://mcp.test/rpc"}"#,
        "Missing required parameter: server_label",
    ),
    (
        r#"{"type":"mcp","server_url":"u","server_label":"l","allowed_tools":"one"}"#,
        "allowed_tools must be a list of strings",
    ),
    (
        r#"{"type":"mcp","server_url":"u","server_label":"l","headers":{"Authorization":7}}"#,
        "headers.Authorization must be a string",
    ),
];

#[test]
fn case_5_mcp_server_configuration() {
    check_servers(&MCP_DESCRIPTORS);
    check_servers(&MCP_MAPPING);
    check_errors(&MCP_FAILURES);
    map_openai_mcp_server_tool(&json(r#"{"server_url":"u","server_label":"l"}"#)).unwrap();
}

const ADVISOR_SHAPES: [Pair; 2] = [
    (
        r#"{"type":"advisor_20260301","model":"claude-opus-5-5","max_uses":3,"caching":{"type":"ephemeral"}}"#,
        r#"[{"type":"advisor_20260301","name":"advisor","model":"claude-opus-5-5","max_uses":3,"caching":{"type":"ephemeral"}}]"#,
    ),
    (
        r#"{"type":"advisor_20260301","model":"","max_uses":null,"caching":null}"#,
        r#"[{"type":"advisor_20260301","name":"advisor","model":""}]"#,
    ),
];

const ADVISOR_FAILURES: [Pair; 3] = [
    (
        r#"{"type":"advisor_20260301"}"#,
        "Advisor tool must have a valid model",
    ),
    (
        r#"{"type":"advisor_20260301","model":7}"#,
        "Advisor tool must have a valid model",
    ),
    (
        r#"{"type":"unknown_20260101","model":"m"}"#,
        "Unsupported tool type: unknown_20260101",
    ),
];

#[test]
fn case_6_advisor_tool() {
    check_tools(&ADVISOR_SHAPES);
    check_errors(&ADVISOR_FAILURES);
}

const CACHE_PRECEDENCE: [Pair; 3] = [
    (
        r#"{"type":"function","function":{"name":"cached","cache_control":{"ttl":"1h"}},"cache_control":{"type":"ephemeral"}}"#,
        r#"[{"name":"cached","input_schema":{"type":"object","properties":{}},"type":"custom","cache_control":{"type":"ephemeral"}}]"#,
    ),
    (
        r#"{"type":"function","function":{"name":"nested","cache_control":{"type":"ephemeral"}}}"#,
        r#"[{"name":"nested","input_schema":{"type":"object","properties":{}},"type":"custom","cache_control":{"type":"ephemeral"}}]"#,
    ),
    (
        r#"{"type":"web_search_20250305","name":"s","function":{"cache_control":{"type":"ephemeral"}}}"#,
        r#"[{"type":"web_search_20250305","name":"s","function":{"cache_control":{"type":"ephemeral"}},"cache_control":{"type":"ephemeral"}}]"#,
    ),
];

const CALLER_PRECEDENCE: [Pair; 2] = [
    (
        r#"{"type":"function","function":{"name":"callers","allowed_callers":["code_execution_20250825"]},"allowed_callers":["direct"]}"#,
        r#"[{"name":"callers","input_schema":{"type":"object","properties":{}},"type":"custom","allowed_callers":["direct"]}]"#,
    ),
    (
        r#"{"type":"function","function":{"name":"nested-callers","allowed_callers":[]}}"#,
        r#"[{"name":"nested-callers","input_schema":{"type":"object","properties":{}},"type":"custom","allowed_callers":[]}]"#,
    ),
];

const EXAMPLE_PRECEDENCE: [Pair; 3] = [
    (
        r#"{"type":"function","function":{"name":"fallback","input_examples":[{"q":1}]},"input_examples":{"bad":1}}"#,
        r#"[{"name":"fallback","input_schema":{"type":"object","properties":{}},"type":"custom","input_examples":[{"q":1}]}]"#,
    ),
    (
        r#"{"type":"function","function":{"name":"top-examples","input_examples":[{"q":1}]},"input_examples":[{"q":2}]}"#,
        r#"[{"name":"top-examples","input_schema":{"type":"object","properties":{}},"type":"custom","input_examples":[{"q":2}]}]"#,
    ),
    (
        r#"{"type":"function","function":{"name":"ignored"},"input_examples":{"bad":1}}"#,
        r#"[{"name":"ignored","input_schema":{"type":"object","properties":{}},"type":"custom"}]"#,
    ),
];

const OPTION_FAILURES: [Pair; 2] = [
    (
        r#"{"type":"function","function":{"name":"x","allowed_callers":[1]}}"#,
        "allowed_callers must be a list of strings",
    ),
    (
        r#"{"type":"function","function":{"name":"x"},"allowed_callers":"direct"}"#,
        "allowed_callers must be a list of strings",
    ),
];

#[test]
fn case_7_cache_examples_callers() {
    check_tools(&CACHE_PRECEDENCE);
    check_tools(&CALLER_PRECEDENCE);
    check_tools(&EXAMPLE_PRECEDENCE);
    check_errors(&OPTION_FAILURES);
}

const DEFER_PRECEDENCE: [Pair; 3] = [
    (
        r#"{"type":"function","function":{"name":"deferred","defer_loading":false}}"#,
        r#"[{"name":"deferred","input_schema":{"type":"object","properties":{}},"type":"custom","defer_loading":false}]"#,
    ),
    (
        r#"{"type":"function","function":{"name":"top-defer"},"defer_loading":true}"#,
        r#"[{"name":"top-defer","input_schema":{"type":"object","properties":{}},"type":"custom","defer_loading":true}]"#,
    ),
    (
        r#"{"type":"function","function":{"name":"nested-defer","defer_loading":null},"defer_loading":null}"#,
        r#"[{"name":"nested-defer","input_schema":{"type":"object","properties":{}},"type":"custom"}]"#,
    ),
];

const DEFER_SUPPRESSION: [Pair; 4] = [
    (
        r#"{"type":"computer_20241022","function":{"parameters":{"display_width_px":1,"display_height_px":2},"defer_loading":true,"allowed_callers":["direct"]}}"#,
        r#"[{"type":"computer_20241022","name":"computer","display_width_px":1,"display_height_px":2}]"#,
    ),
    (
        r#"{"type":"computer_20241023","function":{"parameters":{"display_width_px":1,"display_height_px":2},"defer_loading":true}}"#,
        r#"[{"type":"computer_20241023","name":"computer","display_width_px":1,"display_height_px":2,"defer_loading":true}]"#,
    ),
    (
        r#"{"type":"web_search_20250305","name":"s","defer_loading":true,"allowed_callers":["direct"]}"#,
        r#"[{"type":"web_search_20250305","name":"s","defer_loading":true,"allowed_callers":["direct"]}]"#,
    ),
    (
        r#"{"type":"bash_20250124","name":"shell","eager_input_streaming":"yes"}"#,
        r#"[{"type":"bash_20250124","name":"shell","eager_input_streaming":"yes"}]"#,
    ),
];

const DEFER_FAILURES: [Pair; 2] = [
    (
        r#"{"type":"function","function":{"name":"x","defer_loading":"yes"}}"#,
        "defer_loading must be a boolean",
    ),
    (
        r#"{"type":"function","function":{"name":"x"},"defer_loading":1}"#,
        "defer_loading must be a boolean",
    ),
];

#[test]
fn case_8_eager_input_streaming_defer_loading() {
    check_tools(&DEFER_PRECEDENCE);
    check_tools(&DEFER_SUPPRESSION);
    check_errors(&DEFER_FAILURES);
}

const NAMESPACE_EXPANSION: [Pair; 1] = [(
    r#"{"type":"namespace","tools":[{"input_schema":{"type":"object","x":1},"name":"native","type":"custom","cache_control":{"type":"ephemeral"}},{"name":"flat","description":"d","parameters":{"properties":{"a":{"type":"string"}}},"cache_control":{"type":"ephemeral"}},{"type":"function","function":{"name":"nested","cache_control":{"type":"ephemeral"}}},{"type":"web_search_20250305"},{"description":"skipped"}]}"#,
    r#"[{"input_schema":{"type":"object","x":1},"name":"native","type":"custom","cache_control":{"type":"ephemeral"}},{"name":"flat","input_schema":{"type":"object","properties":{"a":{"type":"string"}}},"type":"custom","description":"d","cache_control":{"type":"ephemeral"}},{"name":"nested","input_schema":{"type":"object","properties":{}},"type":"custom","cache_control":{"type":"ephemeral"}}]"#,
)];

const NATIVE_PASSTHROUGH: [Pair; 2] = [
    (
        r#"{"type":"custom","name":"kept","input_schema":{"type":"object","unknown":true},"defer_loading":true}"#,
        r#"[{"type":"custom","name":"kept","input_schema":{"type":"object","unknown":true},"defer_loading":true}]"#,
    ),
    (
        r#"{"type":"custom","name":"first","input_schema":{}}"#,
        r#"[{"type":"custom","name":"first","input_schema":{}}]"#,
    ),
];

const NAMESPACE_FAILURES: [Pair; 3] = [
    (
        r#"{"description":"nameless"}"#,
        "Missing required parameter: type",
    ),
    (
        r#"{"type":"namespace","tools":"bad"}"#,
        "tools must be a list",
    ),
    ("7", "tool must be a JSON object"),
];

#[test]
fn case_9_namespace_and_native_tools() {
    check_tools(&NAMESPACE_EXPANSION);
    check_tools(&NATIVE_PASSTHROUGH);
    check_tools_errors(&NAMESPACE_FAILURES);
    namespace_members_keep_their_own_order();
}

// one pass over each list, so the test reads the two orders a caller sees
fn mapped_names(mapping: &ToolMapping) -> (Vec<String>, Vec<String>) {
    let tools = mapping
        .tools
        .iter()
        .filter_map(|tool| match tool {
            ToolDefinition::Function(function) => Some(function.name.clone()),
            ToolDefinition::Computer(computer) => Some(computer.name.clone()),
            ToolDefinition::Hosted(hosted) => Some(hosted.name.clone()),
            ToolDefinition::Advisor(_) | ToolDefinition::Native(_) => None,
        })
        .collect();
    let servers = mapping
        .mcp_servers
        .iter()
        .filter_map(|server| match server {
            McpServerTool::Mapped(mapped) => Some(mapped.name.clone()),
            McpServerTool::Descriptor(fields) => {
                fields.get("name").and_then(Json::as_str).map(str::to_owned)
            }
        })
        .collect();
    (tools, servers)
}

fn namespace_members_keep_their_own_order() {
    // a nested MCP descriptor has no name and no function, so the namespace loop skips it
    let skipped = mapping(r#"{"type":"namespace","tools":[{"name":"flat"},{"type":"mcp"}]}"#);
    assert_eq!(skipped.tools.len(), 1);
    assert_eq!(skipped.mcp_servers.len(), 0);
    let ordered = map_tools(&[
        json(r#"{"type":"mcp","server_url":"u","server_label":"first"}"#),
        json(r#"{"type":"function","function":{"name":"one"}}"#),
        json(r#"{"type":"mcp","server_url":"u","server_label":"second"}"#),
        json(r#"{"type":"function","function":{"name":"two"}}"#),
    ])
    .unwrap();
    let (tools, servers) = mapped_names(&ordered);
    assert_eq!(tools, ["one", "two"]);
    assert_eq!(servers, ["first", "second"]);
}

const SEARCH_HOSTED: [Pair; 2] = [
    (
        r#"{"type":"tool_search_tool_regex_20251119","name":"search","max_uses":2,"cache_control":{"type":"ephemeral"},"defer_loading":true,"allowed_callers":["direct"],"input_examples":[{"q":1}]}"#,
        r#"[{"type":"tool_search_tool_regex_20251119","name":"search","max_uses":2,"cache_control":{"type":"ephemeral"},"defer_loading":true,"allowed_callers":["direct"],"input_examples":[{"q":1}]}]"#,
    ),
    (
        r#"{"type":"tool_search_tool_bm25_20251119","name":""}"#,
        r#"[{"type":"tool_search_tool_bm25_20251119","name":""}]"#,
    ),
];

const SEARCH_SUPPRESSED: [Pair; 1] = [(
    r#"{"type":"tool_search_tool_regex_20251119","name":"search","function":{"cache_control":{"type":"ephemeral"},"defer_loading":true,"allowed_callers":["direct"]}}"#,
    r#"[{"type":"tool_search_tool_regex_20251119","name":"search","function":{"cache_control":{"type":"ephemeral"},"defer_loading":true,"allowed_callers":["direct"]}}]"#,
)];

const SEARCH_FAILURES: [Pair; 2] = [
    (
        r#"{"type":"tool_search_tool_regex_20251119","function":{}}"#,
        "Missing required parameter: name",
    ),
    (
        r#"{"type":"tool_search_tool_bm25_20251119"}"#,
        "Missing required parameter: name",
    ),
];

#[test]
fn case_10_search_tool_branch_shadowing() {
    check_tools(&SEARCH_HOSTED);
    check_tools(&SEARCH_SUPPRESSED);
    check_errors(&SEARCH_FAILURES);
    let error = tools_failure(r#"{"type":"tool_search_tool_bm25_20251119"}"#);
    assert!(matches!(error, ToolsError::Missing("name")));
}
