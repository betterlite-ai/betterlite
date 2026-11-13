use rstest::rstest;

use crate::bedrock_tools::capabilities::CachePointSupport;
use crate::bedrock_tools::name::ToolNameMap;

use super::prelude::{Harness, Settings, field, json, list, messages};

const NATIVE_AND_FUNCTION_TOOLS: &str = r#"[
    {"toolSpec": {"name": "native", "description": "kept", "inputSchema": {"json": {"type": "object"}}}},
    {"type": "function", "function": {"name": "get weather", "description": "Get the weather", "parameters": {"type": "object"}}},
    {"systemTool": {"name": "nova_grounding"}}
]"#;

const GET_WEATHER_TOOL: &str = r#"[
    {"type": "function", "function": {"name": "get_weather", "description": "Get the weather", "parameters": {"type": "object"}}}
]"#;

const SANITIZED_TOOL: &str = r#"[
    {"type": "function", "function": {"name": "get weather", "description": "Get the weather", "parameters": {"type": "object"}}}
]"#;

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_native_blocks_pass_through_and_functions_become_tool_specs()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"go"}]"#),
            &list(NATIVE_AND_FUNCTION_TOOLS),
            &Settings::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.tools,
        list(
            r#"[
                {"toolSpec": {"name": "native", "description": "kept", "inputSchema": {"json": {"type": "object"}}}},
                {"toolSpec": {"name": "get_weather", "description": "Get the weather",
                    "inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}}},
                {"systemTool": {"name": "nova_grounding"}}
            ]"#
        )
    );
    assert_eq!(converted.names.len(), 1);
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_message_tool_calls_and_tools_share_one_reverse_map()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":"go"},
                    {"role":"assistant","content":null,"tool_calls":[
                        {"id":"call_1","type":"function","function":{"name":"other tool","arguments":"{}"}}
                    ]}]"#,
            ),
            &list(SANITIZED_TOOL),
            &Settings::default(),
        )
        .await
        .unwrap();
    let mut expected = ToolNameMap::new();
    expected.insert("get_weather".to_owned(), "get weather".to_owned());
    expected.insert("other_tool".to_owned(), "other tool".to_owned());
    assert_eq!(converted.names, expected);
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_valid_tool_name_adds_no_reverse_mapping() {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"go"}]"#),
            &list(GET_WEATHER_TOOL),
            &Settings::default(),
        )
        .await
        .unwrap();
    assert!(converted.names.is_empty());
}

#[rstest]
#[case::strict_flagged(true, true)]
#[case::strict_not_flagged(false, false)]
#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_strict_tools_follow_the_injected_flag(
    #[case] flag: bool,
    #[case] emitted: bool,
) {
    let harness = Harness::new();
    let tools = list(
        r#"[{"type":"function","function":{
            "name":"get_weather","description":"d","parameters":{"type":"object"},"strict":true
        }}]"#,
    );
    let settings = if flag {
        Settings::default().with_strict_tools()
    } else {
        Settings::default()
    };
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"go"}]"#),
            &tools,
            &settings,
        )
        .await
        .unwrap();
    let spec = field(converted.tools.first().unwrap(), "toolSpec").unwrap();
    assert_eq!(field(spec, "strict").is_some(), emitted);
}

#[rstest]
#[case::unsupported(CachePointSupport::Unsupported, None)]
#[case::short_ttl(CachePointSupport::ShortTtl, Some(false))]
#[case::extended_ttl(CachePointSupport::ExtendedTtl, Some(true))]
#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_tool_cache_control_follows_the_model_flag(
    #[case] support: CachePointSupport,
    #[case] expected: Option<bool>,
) {
    let harness = Harness::new();
    let tools = list(
        r#"[{"type":"function","function":{"name":"get_weather","description":"d","parameters":{"type":"object"}},
             "cache_control":{"type":"ephemeral","ttl":"1h"}}]"#,
    );
    let settings = Settings::default()
        .with_model("anthropic.claude-sonnet-4-5-20250929-v1:0")
        .with_cache_points(support);
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"go"}]"#),
            &tools,
            &settings,
        )
        .await
        .unwrap();
    let point = converted
        .tools
        .iter()
        .find_map(|tool| field(tool, "cachePoint"));
    match expected {
        None => assert!(point.is_none()),
        Some(false) => {
            assert_eq!(point.unwrap(), &json(r#"{"type":"default"}"#));
        }
        Some(true) => {
            assert_eq!(point.unwrap(), &json(r#"{"type":"default","ttl":"1h"}"#));
        }
    }
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_parallel_conversions_keep_their_own_reverse_names()
 {
    let harness = Harness::new();
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let first_tools = list(
        r#"[{"type":"function","function":{"name":"first tool","parameters":{"type":"object"}}}]"#,
    );
    let second_tools = list(
        r#"[{"type":"function","function":{"name":"second tool","parameters":{"type":"object"}}}]"#,
    );
    let settings = Settings::default();
    let (first_result, second_result) = tokio::join!(
        harness.convert(&messages, &first_tools, &settings),
        harness.convert(&messages, &second_tools, &settings),
    );
    let first = first_result.unwrap();
    let second = second_result.unwrap();
    assert_eq!(first.names.len(), 1);
    assert_eq!(
        first.names.get("first_tool").map(String::as_str),
        Some("first tool")
    );
    assert_eq!(second.names.len(), 1);
    assert_eq!(
        second.names.get("second_tool").map(String::as_str),
        Some("second tool")
    );
}
