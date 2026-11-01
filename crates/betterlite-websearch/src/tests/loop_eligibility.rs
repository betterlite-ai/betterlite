use crate::loop_eligibility::{
    CHAT_COMPLETION_SURFACE, RESPONSES_AGENTIC_SURFACE, WebSearchInterceptionState,
};
use crate::tools::Tool;

const ANTHROPIC_RESPONSE_WITH_THINKING: &str = r#"{"content":[
    {"type":"thinking","thinking":"reason","signature":"sig"},
    {"type":"tool_use","id":"call-1","name":"litellm_web_search","input":{"query":"rust"}}
]}"#;

const ANTHROPIC_RESPONSE_WITH_REDACTED_THINKING: &str = r#"{"content":[
    {"type":"redacted_thinking","data":"abc"},
    {"type":"tool_use","id":"call-1","name":"litellm_web_search","input":{"query":"rust"}}
]}"#;

const ANTHROPIC_RESPONSE_NO_TOOL: &str = r#"{"content":[{"type":"text","text":"hi"}]}"#;

const CHAT_RESPONSE_WITH_CALL: &str = r#"{"choices":[{"message":{"tool_calls":[
    {"id":"call-1","type":"function","function":{"name":"litellm_web_search","arguments":"{\"query\":\"rust\"}"}}
]}}]}"#;

const RESPONSES_RESPONSE_WITH_CALL: &str = r#"{"output":[
    {"type":"function_call","call_id":"call-1","name":"litellm_web_search","arguments":{"query":"rust"}}
]}"#;

fn search_tool() -> Tool {
    super::tool(Some("function"), Some("litellm_web_search"), None, false)
}

fn native_search_tool() -> Tool {
    super::tool(Some("web_search_20250305"), Some("web_search"), None, false)
}

#[test]
fn dispatcher_routes_to_chat_completions_gate_by_surface_kwarg() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .async_should_run_agentic_loop(
            CHAT_RESPONSE_WITH_CALL,
            "gpt-4",
            &[],
            Some(&[search_tool()]),
            false,
            "openai",
            Some(CHAT_COMPLETION_SURFACE),
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.response_format, "openai");
}

#[test]
fn dispatcher_routes_to_responses_gate_by_surface_kwarg() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .async_should_run_agentic_loop(
            RESPONSES_RESPONSE_WITH_CALL,
            "gpt-4",
            &[],
            Some(&[search_tool()]),
            false,
            "openai",
            Some(RESPONSES_AGENTIC_SURFACE),
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.response_format, "responses");
}

#[test]
fn dispatcher_defaults_to_anthropic_gate_for_absent_surface() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .async_should_run_agentic_loop(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            "claude",
            &[],
            Some(&[search_tool()]),
            false,
            "anthropic",
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.response_format, "anthropic");
}

#[test]
fn dispatcher_defaults_to_anthropic_gate_for_unrecognized_surface() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .async_should_run_agentic_loop(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            "claude",
            &[],
            Some(&[search_tool()]),
            false,
            "anthropic",
            Some("unknown"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.response_format, "anthropic");
}

#[test]
fn anthropic_gate_extracts_thinking_blocks_from_response_content() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap()
        .unwrap();
    let thinking_blocks = dict.thinking_blocks.unwrap();
    assert_eq!(thinking_blocks.len(), 1);
    assert_eq!(thinking_blocks[0]["type"], "thinking");
}

#[test]
fn anthropic_gate_extracts_redacted_thinking_blocks() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_REDACTED_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap()
        .unwrap();
    let thinking_blocks = dict.thinking_blocks.unwrap();
    assert_eq!(thinking_blocks.len(), 1);
    assert_eq!(thinking_blocks[0]["type"], "redacted_thinking");
}

#[test]
fn anthropic_gate_returns_empty_thinking_blocks_when_none_present() {
    let state = WebSearchInterceptionState::default();
    let response = r#"{"content":[
        {"type":"tool_use","id":"call-1","name":"litellm_web_search","input":{"query":"rust"}}
    ]}"#;
    let dict = state
        .anthropic_gate(response, Some(&[search_tool()]), false, "anthropic")
        .unwrap()
        .unwrap();
    assert_eq!(dict.thinking_blocks, Some(Vec::new()));
}

#[test]
fn chat_completion_gate_detects_tool_calls_via_openai_dialect() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .chat_completion_gate(
            CHAT_RESPONSE_WITH_CALL,
            Some(&[search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.tool_calls.len(), 1);
    assert_eq!(dict.response_format, "openai");
}

#[test]
fn chat_completion_gate_rejects_native_anthropic_tool_type() {
    let state = WebSearchInterceptionState::default();
    let result = state
        .chat_completion_gate(
            CHAT_RESPONSE_WITH_CALL,
            Some(&[native_search_tool()]),
            false,
            "openai",
        )
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn responses_gate_detects_tool_calls_via_responses_dialect() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .responses_gate(
            RESPONSES_RESPONSE_WITH_CALL,
            Some(&[search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.tool_calls.len(), 1);
    assert_eq!(dict.response_format, "responses");
}

#[test]
fn responses_gate_accepts_native_web_search_prefixed_tool_type() {
    let state = WebSearchInterceptionState::default();
    let dict = state
        .responses_gate(
            RESPONSES_RESPONSE_WITH_CALL,
            Some(&[native_search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    assert_eq!(dict.response_format, "responses");
}

#[test]
fn provider_filter_blocks_a_provider_outside_the_enabled_list() {
    let state = WebSearchInterceptionState {
        enabled_providers: Some(vec!["bedrock".to_owned()]),
    };
    let result = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn provider_filter_allows_a_provider_present_in_the_enabled_list() {
    let state = WebSearchInterceptionState {
        enabled_providers: Some(vec!["anthropic".to_owned()]),
    };
    let result = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap();
    assert!(result.is_some());
}

#[test]
fn provider_filter_allows_every_provider_when_unset() {
    let state = WebSearchInterceptionState::default();
    let result = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "any_provider",
        )
        .unwrap();
    assert!(result.is_some());
}

#[test]
fn tool_check_requires_a_matching_web_search_tool_per_gate() {
    let state = WebSearchInterceptionState::default();
    assert!(
        state
            .anthropic_gate(ANTHROPIC_RESPONSE_WITH_THINKING, None, false, "anthropic")
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .anthropic_gate(
                ANTHROPIC_RESPONSE_WITH_THINKING,
                Some(&[]),
                false,
                "anthropic"
            )
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .chat_completion_gate(
                CHAT_RESPONSE_WITH_CALL,
                Some(&[native_search_tool()]),
                false,
                "openai"
            )
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .responses_gate(RESPONSES_RESPONSE_WITH_CALL, None, false, "openai")
            .unwrap()
            .is_none()
    );
}

#[test]
fn tool_check_blocks_a_response_with_no_tool_use_even_with_a_search_tool_offered() {
    let state = WebSearchInterceptionState::default();
    let result = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_NO_TOOL,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn return_format_includes_required_keys_for_every_gate() {
    let state = WebSearchInterceptionState::default();
    let anthropic = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap()
        .unwrap();
    assert_eq!(anthropic.tool_type, "websearch");
    assert_eq!(anthropic.provider, "anthropic");
    assert_eq!(anthropic.response_format, "anthropic");
    assert!(anthropic.thinking_blocks.is_some());

    let chat = state
        .chat_completion_gate(
            CHAT_RESPONSE_WITH_CALL,
            Some(&[search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    assert_eq!(chat.tool_type, "websearch");
    assert_eq!(chat.provider, "openai");
    assert_eq!(chat.response_format, "openai");
    assert!(chat.thinking_blocks.is_none());

    let responses = state
        .responses_gate(
            RESPONSES_RESPONSE_WITH_CALL,
            Some(&[search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    assert_eq!(responses.tool_type, "websearch");
    assert_eq!(responses.response_format, "responses");
    assert!(responses.thinking_blocks.is_none());
}

#[test]
fn return_format_omits_thinking_blocks_keys_for_chat_and_responses_serialization() {
    let state = WebSearchInterceptionState::default();
    let chat = state
        .chat_completion_gate(
            CHAT_RESPONSE_WITH_CALL,
            Some(&[search_tool()]),
            false,
            "openai",
        )
        .unwrap()
        .unwrap();
    let encoded_chat = serde_json::to_value(&chat).unwrap();
    assert!(encoded_chat.get("thinking_blocks").is_none());

    let anthropic = state
        .anthropic_gate(
            ANTHROPIC_RESPONSE_WITH_THINKING,
            Some(&[search_tool()]),
            false,
            "anthropic",
        )
        .unwrap()
        .unwrap();
    let encoded_anthropic = serde_json::to_value(&anthropic).unwrap();
    assert!(encoded_anthropic.get("thinking_blocks").is_some());
}
