#![cfg(test)]

use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::MockPlan;
use betterlite_config::sections::mock_response::{
    MockChatChoice, MockChatMessage, MockChatObject, MockFunctionCall, MockResponse, MockToolCall,
    MockUsage, UsageDefaults,
};
use betterlite_openai::chat::response::ChatUsage;
use rstest::rstest;

use super::{DEFAULT_CONTENT, response};

fn defaults() -> UsageDefaults {
    UsageDefaults {
        prompt_tokens: 10,
        completion_tokens: 20,
    }
}

fn calls() -> Vec<MockToolCall> {
    vec![MockToolCall {
        id: String::from("call-1"),
        kind: String::from("function"),
        function: MockFunctionCall {
            name: String::from("lookup"),
            arguments: String::from("{}"),
        },
    }]
}

struct ChatCase {
    text: Option<&'static str>,
    choice_count: Option<u32>,
    prompt_tokens: Option<u64>,
    tools: bool,
    expected_error: Option<MockError>,
    choices: usize,
    content: Option<&'static str>,
    prompt: u64,
    completion: u64,
}

#[rstest]
#[case::default_request(ChatCase {
    text: None,
    choice_count: None,
    prompt_tokens: None,
    tools: false,
    expected_error: None,
    choices: 1,
    content: Some(DEFAULT_CONTENT),
    prompt: 10,
    completion: 20,
})]
#[case::string_content(ChatCase {
    text: Some("hello"),
    choice_count: None,
    prompt_tokens: None,
    tools: false,
    expected_error: None,
    choices: 1,
    content: Some("hello"),
    prompt: 10,
    completion: 20,
})]
#[case::n_choices(ChatCase {
    text: Some("abc"),
    choice_count: Some(2),
    prompt_tokens: None,
    tools: false,
    expected_error: None,
    choices: 2,
    content: Some("abc"),
    prompt: 10,
    completion: 20,
})]
#[case::n_zero(ChatCase {
    text: Some("abc"),
    choice_count: Some(0),
    prompt_tokens: None,
    tools: false,
    expected_error: None,
    choices: 0,
    content: None,
    prompt: 10,
    completion: 20,
})]
#[case::prompt_tokens(ChatCase {
    text: Some("abc"),
    choice_count: None,
    prompt_tokens: Some(7),
    tools: false,
    expected_error: None,
    choices: 1,
    content: Some("abc"),
    prompt: 7,
    completion: 20,
})]
#[case::tool_calls(ChatCase {
    text: Some("abc"),
    choice_count: None,
    prompt_tokens: None,
    tools: true,
    expected_error: None,
    choices: 1,
    content: Some("abc"),
    prompt: 10,
    completion: 20,
})]
#[case::rate_limit(ChatCase {
    text: Some("litellm.RateLimitError"),
    choice_count: None,
    prompt_tokens: None,
    tools: false,
    expected_error: Some(MockError::RateLimit),
    choices: 0,
    content: None,
    prompt: 0,
    completion: 0,
})]
#[case::context_window(ChatCase {
    text: Some("litellm.ContextWindowExceededError"),
    choice_count: None,
    prompt_tokens: None,
    tools: false,
    expected_error: Some(MockError::ContextWindowExceeded),
    choices: 0,
    content: None,
    prompt: 0,
    completion: 0,
})]
#[case::internal_server(ChatCase {
    text: Some("litellm.InternalServerError"),
    choice_count: None,
    prompt_tokens: None,
    tools: false,
    expected_error: Some(MockError::InternalServerError),
    choices: 0,
    content: None,
    prompt: 0,
    completion: 0,
})]
#[case::content_filter(ChatCase {
    text: Some("Exception: content_filter_policy: blocked"),
    choice_count: None,
    prompt_tokens: None,
    tools: false,
    expected_error: Some(MockError::ContentFilter),
    choices: 0,
    content: None,
    prompt: 0,
    completion: 0,
})]
#[case::streaming_error(ChatCase {
    text: Some("Exception: mock_streaming_error"),
    choice_count: None,
    prompt_tokens: None,
    tools: false,
    expected_error: Some(MockError::Streaming),
    choices: 0,
    content: None,
    prompt: 0,
    completion: 0,
})]
fn mock_calls_behavior_tests_behavior(#[case] case: ChatCase) {
    let ChatCase {
        text,
        choice_count,
        prompt_tokens,
        tools,
        expected_error,
        choices,
        content,
        prompt,
        completion,
    } = case;
    let response_value = text.map(|value| MockResponse::Text(value.to_owned()));
    let calls = calls();
    let plan = MockPlan {
        response: response_value.as_ref(),
        tool_calls: tools.then_some(calls.as_slice()),
        choice_count,
        prompt_tokens,
    };
    let result = response(&plan, defaults(), "gpt-4o", "chatcmpl-test", 1_700_000_000);
    if let Some(error) = expected_error {
        assert_eq!(result.err(), Some(error));
        return;
    }
    let answer = result.expect("mock response");
    assert_eq!(answer.id, "chatcmpl-test");
    assert_eq!(answer.created, 1_700_000_000);
    assert_eq!(answer.model, "gpt-4o");
    assert_eq!(answer.object, "chat.completion");
    assert_eq!(answer.choices.len(), choices);
    if let Some(expected_content) = content {
        assert_eq!(
            answer.choices[0].message.content.as_deref(),
            Some(expected_content)
        );
    }
    assert_eq!(answer.usage.prompt_tokens, prompt);
    assert_eq!(answer.usage.completion_tokens, completion);
    assert_eq!(answer.usage.total_tokens, prompt.saturating_add(completion));
    if tools {
        let attached = answer.choices[0]
            .message
            .tool_calls
            .as_ref()
            .expect("tool calls");
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].id, "call-1");
    }
}

#[test]
fn a_numeric_mock_response_is_wrong_for_chat() {
    let value = MockResponse::Numbers(vec![
        serde_json::Number::from_f64(0.1).expect("number"),
        serde_json::Number::from_f64(0.2).expect("number"),
    ]);
    let plan = MockPlan {
        response: Some(&value),
        tool_calls: None,
        choice_count: None,
        prompt_tokens: None,
    };
    assert_eq!(
        response(&plan, defaults(), "gpt-4o", "id", 1).err(),
        Some(MockError::WrongType {
            field: "mock_response"
        })
    );
}

#[test]
fn tool_calls_need_a_choice_to_attach_to() {
    let value = MockResponse::Text(String::from("abc"));
    let calls = calls();
    let plan = MockPlan {
        response: Some(&value),
        tool_calls: Some(calls.as_slice()),
        choice_count: Some(0),
        prompt_tokens: None,
    };
    assert_eq!(
        response(&plan, defaults(), "gpt-4o", "id", 1).err(),
        Some(MockError::ToolCallsWithoutChoice)
    );
}

fn filled_object() -> MockChatObject {
    MockChatObject {
        id: Some(String::from("given-id")),
        created: Some(5),
        model: Some(String::from("given-model")),
        object: Some(String::from("ignored")),
        system_fingerprint: Some(String::from("fp")),
        service_tier: Some(String::from("default")),
        choices: Some(vec![MockChatChoice {
            index: 2,
            message: MockChatMessage {
                role: String::from("assistant"),
                content: Some(String::from("dict-content")),
                tool_calls: None,
            },
            finish_reason: Some(String::from("length")),
        }]),
        usage: Some(MockUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
        }),
    }
}

#[test]
fn an_object_mock_keeps_given_values() {
    let value = MockResponse::Object(filled_object());
    let plan = MockPlan {
        response: Some(&value),
        tool_calls: None,
        choice_count: None,
        prompt_tokens: None,
    };
    let answer = response(&plan, defaults(), "gpt-4o", "id", 1).expect("mock response");
    assert_eq!(answer.id, "given-id");
    assert_eq!(answer.created, 5);
    assert_eq!(answer.model, "given-model");
    assert_eq!(answer.object, "chat.completion");
    assert_eq!(answer.choices[0].index, 2);
    assert_eq!(
        answer.choices[0].message.content.as_deref(),
        Some("dict-content")
    );
    assert_eq!(answer.choices[0].finish_reason.as_deref(), Some("length"));
    assert_eq!(answer.usage.prompt_tokens, 1);
    assert_eq!(answer.usage.completion_tokens, 2);
    assert_eq!(answer.usage.total_tokens, 3);
}

#[test]
fn an_empty_object_mock_fills_litellm_defaults() {
    let empty = MockChatObject {
        id: None,
        created: None,
        model: None,
        object: None,
        system_fingerprint: None,
        service_tier: None,
        choices: None,
        usage: None,
    };
    let value = MockResponse::Object(empty);
    let plan = MockPlan {
        response: Some(&value),
        tool_calls: None,
        choice_count: None,
        prompt_tokens: None,
    };
    let answer = response(&plan, defaults(), "gpt-4o", "given-id", 9).expect("mock response");
    assert_eq!(answer.id, "given-id");
    assert_eq!(answer.created, 9);
    assert_eq!(answer.model, "gpt-4o");
    assert_eq!(answer.choices.len(), 1);
    assert_eq!(answer.choices[0].index, 0);
    assert_eq!(answer.choices[0].message.role, "assistant");
    assert_eq!(answer.choices[0].message.content, None);
    assert_eq!(answer.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(answer.usage, ChatUsage::default());
}
