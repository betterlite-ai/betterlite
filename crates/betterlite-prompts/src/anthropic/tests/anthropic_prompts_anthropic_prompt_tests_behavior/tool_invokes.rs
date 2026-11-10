use rstest::rstest;

use crate::anthropic::assistant::AssistantError;
use crate::anthropic::error::AnthropicPromptError;

use super::fixtures::{converted, expected, failure};

#[rstest]
#[case::tools_result_round_trip(
    r#"[{"role":"user","content":"weather?"},{"role":"assistant","content":null,"tool_calls":[{"id":"call.1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Boston, MA\"}"}}]},{"role":"tool","tool_call_id":"call.1","content":"sunny"}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"weather?"}]},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"get_weather","input":{"city":"Boston, MA"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"sunny"}]}]"#
)]
#[case::non_function_tool_calls_are_skipped(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"custom","function":{"name":"f","arguments":"{}"}}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"u"}]}]"#
)]
#[case::tool_calls_null_adds_nothing(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":"a","tool_calls":null}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"text","text":"a"}]}]"#
)]
#[case::a_repeated_tool_id_is_emitted_once(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"f","input":{}}]}]"#
)]
#[case::a_tool_id_repeated_in_separate_turns_is_sent_in_both(
    r#"[{"role":"user","content":"first"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call_1","content":"result"},{"role":"user","content":"second"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"first"}]},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"f","input":{}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"result"},{"type":"text","text":"second"}]},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"f","input":{}}]}]"#
)]
#[case::function_call_becomes_a_tool_use(
    r#"[{"role":"user","content":"u"},{"role":"assistant","function_call":{"name":"legacy","arguments":"{\"x\":1}"}}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"tool_use","id":"generated-id","name":"legacy","input":{"x":1}}]}]"#
)]
#[case::function_call_without_name_or_arguments(
    r#"[{"role":"user","content":"u"},{"role":"assistant","function_call":{}}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"tool_use","id":"generated-id","name":"","input":{}}]}]"#
)]
#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior(
    #[case] input: &str,
    #[case] output: &str,
) {
    assert_eq!(converted(input, false).await, expected(output));
}

#[rstest]
#[case::tool_calls_not_a_list(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"tool_calls":{}}]"#,
    AssistantError::Field {
        index: 1,
        field: "tool_calls",
    }
)]
#[case::function_call_not_an_object(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"function_call":"legacy"}]"#,
    AssistantError::Field {
        index: 1,
        field: "function_call",
    }
)]
#[case::web_search_results_not_a_list(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}],"provider_specific_fields":{"web_search_results":{"tool_use_id":"call_1"}}}]"#,
    AssistantError::Field {
        index: 1,
        field: "web_search_results",
    }
)]
#[case::tool_results_not_a_list(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}],"provider_specific_fields":{"tool_results":5}}]"#,
    AssistantError::Field {
        index: 1,
        field: "tool_results",
    }
)]
#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_tool_failures(
    #[case] input: &str,
    #[case] expected_failure: AssistantError,
) {
    let error = failure(input, false).await;
    let AnthropicPromptError::Assistant(assistant) = error else {
        panic!("expected an assistant failure, got {error}");
    };
    assert_eq!(assistant.to_string(), expected_failure.to_string());
}
