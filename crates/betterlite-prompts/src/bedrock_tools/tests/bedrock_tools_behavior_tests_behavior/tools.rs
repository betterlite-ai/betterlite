use crate::arguments::json::{Json, Object};
use crate::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use crate::bedrock_tools::error::BedrockToolError;
use crate::bedrock_tools::name::ToolNameMap;
use crate::bedrock_tools::tools::bedrock_tools;

use super::prelude::{json, json_list};

const UNSUPPORTED: CachePointSupport = CachePointSupport::Unsupported;
const SHORT: CachePointSupport = CachePointSupport::ShortTtl;
const EXTENDED: CachePointSupport = CachePointSupport::ExtendedTtl;

enum Failure {
    NotObject(usize),
    WrongType(&'static str),
}

enum Scenario {
    Success {
        tools: &'static str,
        cache_points: CachePointSupport,
        supports_strict_tools: bool,
        expected: &'static str,
        reverse: &'static str,
    },
    Failure {
        tools: &'static str,
        expected: Failure,
    },
    SharedBudget,
}

#[rstest::rstest]
#[case::copies_native_blocks_and_drops_builtin_tools(Scenario::Success {
    tools: r#"[
        {"toolSpec":{"name":"native","description":"kept","inputSchema":{"json":{"type":"object"}}}},
        {"systemTool":{"name":"nova_grounding"}},
        {"cachePoint":{"type":"default","ttl":"1h"}},
        {"type":"web_search"},
        {"type":"file_search"}
    ]"#,
    cache_points: UNSUPPORTED,
    supports_strict_tools: false,
    expected: r#"[
        {"toolSpec":{"name":"native","description":"kept","inputSchema":{"json":{"type":"object"}}}},
        {"systemTool":{"name":"nova_grounding"}},
        {"cachePoint":{"type":"default","ttl":"1h"}}
    ]"#,
    reverse: "{}",
})]
#[case::projects_an_openai_function_tool(Scenario::Success {
    tools: r#"[{"type":"function","function":{"name":"get_weather","description":"Get the weather","parameters":{"type":"object","title":"dropped","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false},"strict":true}}]"#,
    cache_points: UNSUPPORTED,
    supports_strict_tools: true,
    expected: r#"[{"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false}},"name":"get_weather","description":"Get the weather","strict":true}}]"#,
    reverse: "{}",
})]
#[case::projects_an_anthropic_tool_with_the_name_as_description(Scenario::Success {
    tools: r#"[{"name":"claude_tool","description":"","input_schema":{"type":"object","properties":{"q":{"type":"string"}}}}]"#,
    cache_points: UNSUPPORTED,
    supports_strict_tools: true,
    expected: r#"[{"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{"q":{"type":"string"}},"required":[]}},"name":"claude_tool","description":"claude_tool"}}]"#,
    reverse: "{}",
})]
#[case::numbers_unnamed_tools_by_their_list_index(Scenario::Success {
    tools: r#"[
        {"type":"function","function":{}},
        {"toolSpec":{"name":"n"}},
        {"type":"function","function":{"name":"   "}},
        {"type":"function","function":{"name":null}}
    ]"#,
    cache_points: UNSUPPORTED,
    supports_strict_tools: false,
    expected: r#"[
        {"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"litellm_unnamed_tool_0","description":"litellm_unnamed_tool_0"}},
        {"toolSpec":{"name":"n"}},
        {"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"litellm_unnamed_tool_2","description":"litellm_unnamed_tool_2"}},
        {"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"litellm_unnamed_tool_3","description":"litellm_unnamed_tool_3"}}
    ]"#,
    reverse: "{}",
})]
#[case::uses_the_function_of_a_tool_that_also_has_an_input_schema(Scenario::Success {
    tools: r#"[{"type":"function","function":{"name":"openai"},"input_schema":{"type":"object","properties":{"x":{"type":"string"}}}}]"#,
    cache_points: UNSUPPORTED,
    supports_strict_tools: false,
    expected: r#"[{"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"openai","description":"openai"}}]"#,
    reverse: "{}",
})]
#[case::keeps_the_last_name_for_a_duplicate(Scenario::Success {
    tools: r#"[
        {"type":"function","function":{"name":"dup name"}},
        {"type":"function","function":{"name":"dup_name"}},
        {"type":"function","function":{"name":"dup~name"}}
    ]"#,
    cache_points: UNSUPPORTED,
    supports_strict_tools: false,
    expected: r#"[
        {"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"dup_name","description":"dup_name"}},
        {"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"dup_name","description":"dup_name"}},
        {"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"dup_name","description":"dup_name"}}
    ]"#,
    reverse: r#"{"dup_name":"dup~name"}"#,
})]
#[case::appends_a_cache_point_after_the_tool_spec(Scenario::Success {
    tools: r#"[{"type":"function","function":{"name":"cached"},"cache_control":{"type":"ephemeral","ttl":"1h"}}]"#,
    cache_points: SHORT,
    supports_strict_tools: false,
    expected: r#"[
        {"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"cached","description":"cached"}},
        {"cachePoint":{"type":"default"}}
    ]"#,
    reverse: "{}",
})]
#[case::keeps_a_ttl_for_a_model_that_takes_one(Scenario::Success {
    tools: r#"[{"type":"function","function":{"name":"cached"},"cache_control":{"type":"ephemeral","ttl":"1h"}}]"#,
    cache_points: EXTENDED,
    supports_strict_tools: false,
    expected: r#"[
        {"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"cached","description":"cached"}},
        {"cachePoint":{"type":"default","ttl":"1h"}}
    ]"#,
    reverse: "{}",
})]
#[case::copies_a_native_block_cache_control_untouched(Scenario::Success {
    tools: r#"[{"toolSpec":{"name":"n"},"cache_control":{"type":"ephemeral"}}]"#,
    cache_points: SHORT,
    supports_strict_tools: false,
    expected: r#"[{"toolSpec":{"name":"n"},"cache_control":{"type":"ephemeral"}}]"#,
    reverse: "{}",
})]
#[case::drops_strict_fields_without_model_support(Scenario::Success {
    tools: r#"[{"type":"function","function":{"name":"plain","strict":true,"parameters":{"type":"object","additionalProperties":false}}}]"#,
    cache_points: UNSUPPORTED,
    supports_strict_tools: false,
    expected: r#"[{"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"plain","description":"plain"}}]"#,
    reverse: "{}",
})]
#[case::rejects_a_tool_that_is_not_an_object(Scenario::Failure {
    tools: r#"["junk"]"#,
    expected: Failure::NotObject(0),
})]
#[case::rejects_a_null_function(Scenario::Failure {
    tools: r#"[{"function":null}]"#,
    expected: Failure::WrongType("function"),
})]
#[case::rejects_a_non_string_name(Scenario::Failure {
    tools: r#"[{"type":"function","function":{"name":7}}]"#,
    expected: Failure::WrongType("name"),
})]
#[case::rejects_a_non_string_description(Scenario::Failure {
    tools: r#"[{"type":"function","function":{"description":7}}]"#,
    expected: Failure::WrongType("description"),
})]
#[case::rejects_non_object_parameters(Scenario::Failure {
    tools: r#"[{"type":"function","function":{"parameters":"x"}}]"#,
    expected: Failure::WrongType("parameters"),
})]
#[case::rejects_a_non_boolean_strict(Scenario::Failure {
    tools: r#"[{"type":"function","function":{"strict":1}}]"#,
    expected: Failure::WrongType("strict"),
})]
#[case::rejects_non_object_properties(Scenario::Failure {
    tools: r#"[{"type":"function","function":{"parameters":{"properties":"x"}}}]"#,
    expected: Failure::WrongType("properties"),
})]
#[case::rejects_a_required_list_with_non_strings(Scenario::Failure {
    tools: r#"[{"type":"function","function":{"parameters":{"required":[1]}}}]"#,
    expected: Failure::WrongType("required"),
})]
#[case::shares_one_expansion_budget_across_the_tool_list(Scenario::SharedBudget)]
fn bedrock_tools_behavior_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Success {
            tools,
            cache_points,
            supports_strict_tools,
            expected,
            reverse,
        } => {
            let converted = bedrock_tools(
                &json_list(tools),
                BedrockToolCapabilities {
                    cache_points,
                    supports_strict_tools,
                },
            )
            .unwrap();
            assert_eq!(Json::List(converted.blocks), json(expected));
            assert_eq!(
                converted.names,
                serde_json::from_str::<ToolNameMap>(reverse).unwrap()
            );
        }
        Scenario::SharedBudget => {
            let result = bedrock_tools(
                &large_tool_list(),
                BedrockToolCapabilities {
                    cache_points: UNSUPPORTED,
                    supports_strict_tools: false,
                },
            );
            assert!(matches!(result, Err(BedrockToolError::SchemaBudget { .. })));
        }
        Scenario::Failure { tools, expected } => {
            let result = bedrock_tools(
                &json_list(tools),
                BedrockToolCapabilities {
                    cache_points: UNSUPPORTED,
                    supports_strict_tools: false,
                },
            );
            match expected {
                Failure::NotObject(index) => assert!(matches!(
                    result,
                    Err(BedrockToolError::ToolNotObject { index: actual }) if actual == index
                )),
                Failure::WrongType(field) => assert!(matches!(
                    result,
                    Err(BedrockToolError::WrongType { field: actual }) if actual == field
                )),
            }
        }
    }
}

fn large_tool_list() -> Vec<Json> {
    let mut tools = Vec::with_capacity(3);
    for index in 0_usize..3_usize {
        let property = Json::Object(Object::from_iter([
            ("type".to_owned(), Json::Text("string".to_owned())),
            ("description".to_owned(), Json::Text("x".repeat(4_000_000))),
        ]));
        let schema = Json::Object(Object::from_iter([
            ("type".to_owned(), Json::Text("object".to_owned())),
            (
                "properties".to_owned(),
                Json::Object(Object::from_iter([("body".to_owned(), property)])),
            ),
        ]));
        let function = Json::Object(Object::from_iter([
            ("name".to_owned(), Json::Text(format!("tool_{index}"))),
            ("parameters".to_owned(), schema),
        ]));
        tools.push(Json::Object(Object::from_iter([
            ("type".to_owned(), Json::Text("function".to_owned())),
            ("function".to_owned(), function),
        ])));
    }
    tools
}
