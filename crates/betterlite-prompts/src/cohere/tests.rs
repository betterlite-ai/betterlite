#![cfg(test)]

use rstest::rstest;

use crate::arguments::json::{Json, Object};
use crate::cohere::error::CohereError;
use crate::cohere::history::cohere_messages_pt_v2;
use crate::cohere::prompt::cohere_message_pt;
use crate::cohere::tool_calls::convert_to_cohere_tool_invoke;
use crate::cohere::types::{CohereMessage, CohereTool};
use crate::gemini_tool_invokes::limits::MAX_TOOL_CALLS;
use crate::media::limits::{MAX_MESSAGE_PARTS, MAX_MESSAGES};
use crate::message_content::{Content, ContentPart, Message};

struct V2Case {
    messages: &'static str,
    message: &'static str,
    history: &'static str,
}

struct LegacyCase {
    messages: &'static str,
    prompt: &'static str,
    tool_results: &'static str,
}

struct InvokeCase {
    tool_calls: &'static str,
    expected: &'static str,
}

struct FailureCase {
    messages: &'static str,
    failure: Failure,
}

enum Failure {
    Exact(&'static str),
    Fields(usize, &'static str),
}

#[derive(Clone, Copy)]
enum V2Operation {
    V2,
    Legacy,
}

enum Scenario {
    V2(V2Case),
    Legacy(LegacyCase),
    Invoke(InvokeCase),
    V2Failure(FailureCase),
    LegacyFailure(FailureCase),
    InvokeFailure(InvokeCase, Failure),
    MessageLimit(V2Operation),
    MessageAtLimit(V2Operation),
    InvokeTooManyToolCalls,
    InvokeMaxToolCalls,
    TooManyToolCalls,
    MaxToolCalls,
    TooManyParts,
    PartsAtLimit,
    Preserved(&'static str),
}

fn parse_messages(text: &str) -> Vec<Message> {
    serde_json::from_str(text).unwrap()
}

fn parsed_json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn assert_v2(case: &V2Case) {
    let messages = parse_messages(case.messages);
    let result = cohere_messages_pt_v2(&messages).unwrap();
    assert_eq!(
        serde_json::to_value(&result.message).unwrap(),
        serde_json::to_value(parsed_json(case.message)).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&result.chat_history).unwrap(),
        serde_json::to_value(parsed_json(case.history)).unwrap()
    );
}

fn assert_legacy(case: &LegacyCase) {
    let messages = parse_messages(case.messages);
    let result = cohere_message_pt(&messages).unwrap();
    assert_eq!(result.prompt, case.prompt);
    assert_eq!(
        serde_json::to_value(&result.tool_results).unwrap(),
        serde_json::to_value(parsed_json(case.tool_results)).unwrap()
    );
}

fn assert_invoke(case: &InvokeCase) {
    let tool_calls: Vec<CohereTool> = serde_json::from_str(case.tool_calls).unwrap();
    let invokes = convert_to_cohere_tool_invoke(&tool_calls).unwrap();
    assert_eq!(
        serde_json::to_value(&invokes).unwrap(),
        serde_json::to_value(parsed_json(case.expected)).unwrap()
    );
}

fn assert_failure(case: &FailureCase, legacy: bool) {
    let messages = parse_messages(case.messages);
    let error = if legacy {
        cohere_message_pt(&messages).unwrap_err()
    } else {
        cohere_messages_pt_v2(&messages).unwrap_err()
    };
    assert_error(&error, &case.failure);
}

fn assert_invoke_failure(case: &InvokeCase, failure: &Failure) {
    let tool_calls: Vec<CohereTool> = serde_json::from_str(case.tool_calls).unwrap();
    let error = convert_to_cohere_tool_invoke(&tool_calls).unwrap_err();
    assert_error(&error, failure);
}

fn assert_error(error: &CohereError, failure: &Failure) {
    match failure {
        Failure::Exact(message) => assert_eq!(error.to_string(), *message),
        Failure::Fields(index, path) => {
            assert!(
                matches!(error, CohereError::Fields { index: actual, .. } if actual == index),
                "unexpected {error}"
            );
            assert!(
                error.to_string().contains(path),
                "{error} does not name {path}"
            );
        }
    }
}

const TOOL_CALL: &str =
    r#"{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}"#;

fn tool_calls_message(count: usize) -> Message {
    let calls = vec![TOOL_CALL; count].join(",");
    let mut messages = parse_messages(&format!(
        r#"[{{"role":"assistant","content":"x","tool_calls":[{calls}]}}]"#
    ));
    messages.remove(0)
}

fn tools(count: usize) -> Vec<CohereTool> {
    serde_json::from_str(&format!("[{}]", vec![TOOL_CALL; count].join(","))).unwrap()
}

fn one_message() -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text("a".to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn parts_message(count: usize) -> Message {
    let part = ContentPart {
        kind: "text".to_owned(),
        fields: Object::from([("text".to_owned(), Json::Text("a".to_owned()))]),
    };
    Message {
        role: "user".to_owned(),
        content: Some(Content::Parts(vec![part; count])),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn run_operation(operation: V2Operation, messages: &[Message]) -> Result<(), CohereError> {
    match operation {
        V2Operation::V2 => cohere_messages_pt_v2(messages).map(drop),
        V2Operation::Legacy => cohere_message_pt(messages).map(drop),
    }
}

fn assert_preserved(source: &'static str) {
    let messages = parse_messages(source);
    let untouched = messages.clone();
    let first = cohere_messages_pt_v2(&messages).unwrap();
    let second = cohere_messages_pt_v2(&messages).unwrap();
    drop(cohere_message_pt(&messages).unwrap());
    assert_eq!(messages, untouched);
    assert_eq!(first, second);
}

#[rstest]
#[case::last_user_text_keeps_history(Scenario::V2(V2Case {
    messages: r#"[{"role":"system","content":"be brief"},{"role":"user","content":"hi"},{"role":"assistant","content":"hello"},{"role":"user","content":"what now?"}]"#,
    message: r#""what now?""#,
    history: r#"[{"role":"SYSTEM","message":"be brief"},{"role":"USER","message":"hi"},{"role":"CHATBOT","message":"hello","tool_calls":[]}]"#,
}))]
#[case::empty_history_content_is_skipped(Scenario::V2(V2Case {
    messages: r#"[{"role":"user","content":""},{"role":"system","content":""},{"role":"user","content":"next"}]"#,
    message: r#""next""#,
    history: "[]",
}))]
#[case::empty_tool_call_id_does_not_recover(Scenario::V2(V2Case {
    messages: r#"[{"role":"assistant","content":null,"tool_calls":[{"id":"","type":"function","function":{"name":"ghost","arguments":"{\"a\":1}"}}]},{"role":"tool","tool_call_id":"","content":"x"}]"#,
    message: r#"{"call":{"name":"","parameters":{}},"outputs":[{"result":"x"}]}"#,
    history: "[]",
}))]
#[case::last_message_parts_concatenate_text(Scenario::V2(V2Case {
    messages: r#"[{"role":"user","content":[{"type":"text","text":"one"},{"type":"image_url","image_url":"https://example.test/a.png"},{"type":"text","text":"two"}]}]"#,
    message: r#""onetwo""#,
    history: "[]",
}))]
#[case::last_function_message_returns_text(Scenario::V2(V2Case {
    messages: r#"[{"role":"function","name":"f","content":"result"}]"#,
    message: r#""result""#,
    history: "[]",
}))]
#[case::last_tool_result_recovers_call(Scenario::V2(V2Case {
    messages: r#"[{"role":"user","content":"check the weather"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":"{\"temperature\":22}"}]"#,
    message: r#"{"call":{"name":"get_weather","parameters":{"city":"Paris"}},"outputs":[{"temperature":22}]}"#,
    history: r#"[{"role":"USER","message":"check the weather"}]"#,
}))]
#[case::tool_result_plain_text_is_wrapped(Scenario::V2(V2Case {
    messages: r#"[{"role":"tool","tool_call_id":"call_9","content":"sunny"}]"#,
    message: r#"{"call":{"name":"","parameters":{}},"outputs":[{"result":"sunny"}]}"#,
    history: "[]",
}))]
#[case::tool_result_empty_content_is_an_empty_object(Scenario::V2(V2Case {
    messages: r#"[{"role":"tool","tool_call_id":"call_9","content":""}]"#,
    message: r#"{"call":{"name":"","parameters":{}},"outputs":[{}]}"#,
    history: "[]",
}))]
#[case::tool_result_takes_the_last_matching_tool(Scenario::V2(V2Case {
    messages: r#"[{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"first","arguments":"{\"a\":1}"}},{"id":"call_1","type":"function","function":{"name":"second","arguments":"{\"b\":2}"}}]},{"role":"tool","tool_call_id":"call_1","content":"ok"}]"#,
    message: r#"{"call":{"name":"second","parameters":{"b":2}},"outputs":[{"result":"ok"}]}"#,
    history: "[]",
}))]
#[case::assistant_run_merges_text_and_tool_calls(Scenario::V2(V2Case {
    messages: r#"[{"role":"user","content":"one"},{"role":"user","content":"two"},{"role":"system","content":"s1"},{"role":"system","content":"s2"},{"role":"assistant","content":"call them","tool_calls":[{"id":"call_1","type":"function","function":{"name":"first","arguments":"{\"a\":1}"}},{"id":"call_2","type":"function","function":{"name":"second","arguments":"{}"}}],"function_call":{"name":"legacy","arguments":"{}"}},{"role":"assistant","content":" more"},{"role":"tool","tool_call_id":"call_1","content":"r1"},{"role":"tool","tool_call_id":"call_2","content":"r2"},{"role":"function","name":"legacy_fn","content":"r3"},{"role":"user","content":"final"}]"#,
    message: r#""final""#,
    history: r#"[{"role":"USER","message":"onetwo"},{"role":"SYSTEM","message":"s1s2"},{"role":"CHATBOT","message":"call them more","tool_calls":[{"name":"first","parameters":{"a":1}},{"name":"second","parameters":{}}]},{"role":"TOOL","tool_results":[{"call":{"name":"first","parameters":{"a":1}},"outputs":[{"result":"r1"}]},{"call":{"name":"second","parameters":{}},"outputs":[{"result":"r2"}]},{"call":{"name":"legacy_fn","parameters":{}},"outputs":[{"result":"r3"}]}]}]"#,
}))]
#[case::assistant_without_text_is_dropped_from_history(Scenario::V2(V2Case {
    messages: r#"[{"role":"user","content":"q"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"user","content":"next"}]"#,
    message: r#""next""#,
    history: r#"[{"role":"USER","message":"q"}]"#,
}))]
#[case::assistant_without_tool_calls_gets_an_empty_list(Scenario::V2(V2Case {
    messages: r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a"},{"role":"user","content":"next"}]"#,
    message: r#""next""#,
    history: r#"[{"role":"USER","message":"q"},{"role":"CHATBOT","message":"a","tool_calls":[]}]"#,
}))]
#[case::legacy_prompt_joins_text_and_reports_tools(Scenario::Legacy(LegacyCase {
    messages: r#"[{"role":"user","content":"one"},{"role":"user","content":"two"},{"role":"system","content":"s1"},{"role":"assistant","content":"call them","tool_calls":[{"id":"call_1","type":"function","function":{"name":"first","arguments":"{\"a\":1}"}}],"function_call":{"name":"legacy","arguments":"{}"}},{"role":"tool","tool_call_id":"call_1","content":"r1"},{"role":"function","name":"legacy_fn","content":"r3"},{"role":"user","content":"final"}]"#,
    prompt: "one\n\ntwo\n\ns1\n\ncall them\n\nr3\n\nfinal",
    tool_results: r#"[{"call":{"name":"first","parameters":{"a":1}},"outputs":[{"result":"r1"}]}]"#,
}))]
#[case::legacy_prompt_is_stripped(Scenario::Legacy(LegacyCase {
    messages: r#"[{"role":"user","content":"hi   \n"}]"#,
    prompt: "hi",
    tool_results: "[]",
}))]
#[case::legacy_skips_empty_content(Scenario::Legacy(LegacyCase {
    messages: r#"[{"role":"assistant","content":null},{"role":"assistant","content":[]},{"role":"user","content":""}]"#,
    prompt: "",
    tool_results: "[]",
}))]
#[case::legacy_skips_empty_text_before_content(Scenario::Legacy(LegacyCase {
    messages: r#"[{"role":"assistant","content":""},{"role":"assistant","content":[]},{"role":"user","content":"hi"}]"#,
    prompt: "hi",
    tool_results: "[]",
}))]
#[case::legacy_empty_messages_are_allowed(Scenario::Legacy(LegacyCase {
    messages: "[]",
    prompt: "",
    tool_results: "[]",
}))]
#[case::invoke_keeps_only_function_calls(Scenario::Invoke(InvokeCase {
    tool_calls: r#"[{"id":"a","type":"function","function":{"name":"first","arguments":"{\"a\":1}"}},{"id":"b","type":"other","function":{"name":"skip","arguments":"{}"}},{"id":"c","type":"function","function":{"arguments":"{}"}}]"#,
    expected: r#"[{"name":"first","parameters":{"a":1}},{"name":null,"parameters":{}}]"#,
}))]
#[case::empty_messages_are_refused(Scenario::V2Failure(FailureCase {
    messages: "[]",
    failure: Failure::Exact("the message list is empty"),
}))]
#[case::unknown_roles_are_refused(Scenario::V2Failure(FailureCase {
    messages: r#"[{"role":"user","content":"q"},{"role":"developer","content":"d"},{"role":"user","content":"next"}]"#,
    failure: Failure::Exact("message 1 has role developer, which the Cohere history cannot place"),
}))]
#[case::last_message_must_carry_content(Scenario::V2Failure(FailureCase {
    messages: r#"[{"role":"assistant","content":null}]"#,
    failure: Failure::Exact("message 0 has no content"),
}))]
#[case::history_user_content_must_exist(Scenario::V2Failure(FailureCase {
    messages: r#"[{"role":"user","content":null},{"role":"user","content":"next"}]"#,
    failure: Failure::Exact("message 0 has no content"),
}))]
#[case::text_parts_must_carry_text(Scenario::V2Failure(FailureCase {
    messages: r#"[{"role":"user","content":[{"type":"text"}]}]"#,
    failure: Failure::Exact("message 0 content part 0 text must be a string"),
}))]
#[case::history_arguments_must_be_json(Scenario::V2Failure(FailureCase {
    messages: r#"[{"role":"assistant","content":"calling","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{"}}]},{"role":"user","content":"next"}]"#,
    failure: Failure::Exact("tool call 0 arguments are not valid JSON"),
}))]
#[case::function_messages_need_a_name(Scenario::V2Failure(FailureCase {
    messages: r#"[{"role":"function","content":"r"},{"role":"user","content":"next"}]"#,
    failure: Failure::Exact("message 0 is a function message without a name"),
}))]
#[case::tool_calls_must_be_a_list(Scenario::V2Failure(FailureCase {
    messages: r#"[{"role":"assistant","content":"c","tool_calls":{}},{"role":"user","content":"next"}]"#,
    failure: Failure::Fields(0, "tool_calls"),
}))]
#[case::legacy_refuses_list_content(Scenario::LegacyFailure(FailureCase {
    messages: r#"[{"role":"user","content":[{"type":"text","text":"a"}]}]"#,
    failure: Failure::Exact("message 0 content is a list, which the Cohere text prompt cannot carry"),
}))]
#[case::invoke_needs_a_function(Scenario::InvokeFailure(
    InvokeCase { tool_calls: r#"[{"id":"a","type":"function"}]"#, expected: "" },
    Failure::Exact("tool call 0 has no function"),
))]
#[case::invoke_needs_arguments(Scenario::InvokeFailure(
    InvokeCase { tool_calls: r#"[{"id":"a","type":"function","function":{"name":"f"}}]"#, expected: "" },
    Failure::Exact("tool call 0 has no arguments"),
))]
#[case::invoke_arguments_must_be_json(Scenario::InvokeFailure(
    InvokeCase { tool_calls: r#"[{"id":"a","type":"function","function":{"name":"f","arguments":"oops"}}]"#, expected: "" },
    Failure::Exact("tool call 0 arguments are not valid JSON"),
))]
#[case::v2_refuses_too_many_messages(Scenario::MessageLimit(V2Operation::V2))]
#[case::legacy_refuses_too_many_messages(Scenario::MessageLimit(V2Operation::Legacy))]
#[case::v2_accepts_the_message_limit(Scenario::MessageAtLimit(V2Operation::V2))]
#[case::legacy_accepts_the_message_limit(Scenario::MessageAtLimit(V2Operation::Legacy))]
#[case::tool_calls_are_bounded(Scenario::TooManyToolCalls)]
#[case::the_tool_call_limit_is_accepted(Scenario::MaxToolCalls)]
#[case::invokes_are_bounded(Scenario::InvokeTooManyToolCalls)]
#[case::the_invoke_tool_call_limit_is_accepted(Scenario::InvokeMaxToolCalls)]
#[case::parts_are_bounded(Scenario::TooManyParts)]
#[case::the_part_limit_is_accepted(Scenario::PartsAtLimit)]
#[case::conversions_do_not_change_the_source(Scenario::Preserved(
    r#"[{"role":"user","content":"q"},{"role":"assistant","content":"call","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call_1","content":"r"}]"#,
))]
fn other_prompts_provider_message_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::V2(case) => assert_v2(&case),
        Scenario::Legacy(case) => assert_legacy(&case),
        Scenario::Invoke(case) => assert_invoke(&case),
        Scenario::V2Failure(case) => assert_failure(&case, false),
        Scenario::LegacyFailure(case) => assert_failure(&case, true),
        Scenario::InvokeFailure(case, failure) => assert_invoke_failure(&case, &failure),
        Scenario::MessageLimit(operation) => {
            let messages = vec![one_message(); MAX_MESSAGES.saturating_add(1)];
            assert!(matches!(
                run_operation(operation, &messages),
                Err(CohereError::TooManyMessages { limit }) if limit == MAX_MESSAGES
            ));
        }
        Scenario::MessageAtLimit(operation) => {
            let messages = vec![one_message(); MAX_MESSAGES];
            run_operation(operation, &messages).unwrap();
        }
        Scenario::TooManyToolCalls => {
            let messages = vec![tool_calls_message(MAX_TOOL_CALLS.saturating_add(1))];
            assert!(matches!(
                cohere_messages_pt_v2(&messages),
                Err(CohereError::TooManyToolCalls { limit }) if limit == MAX_TOOL_CALLS
            ));
        }
        Scenario::MaxToolCalls => {
            let messages = vec![tool_calls_message(MAX_TOOL_CALLS)];
            drop(cohere_message_pt(&messages).unwrap());
        }
        Scenario::InvokeTooManyToolCalls => {
            assert!(matches!(
                convert_to_cohere_tool_invoke(&tools(MAX_TOOL_CALLS.saturating_add(1))),
                Err(CohereError::TooManyToolCalls { limit }) if limit == MAX_TOOL_CALLS
            ));
        }
        Scenario::InvokeMaxToolCalls => {
            let invokes = convert_to_cohere_tool_invoke(&tools(MAX_TOOL_CALLS)).unwrap();
            assert_eq!(invokes.len(), MAX_TOOL_CALLS);
        }
        Scenario::TooManyParts => {
            let messages = vec![parts_message(MAX_MESSAGE_PARTS.saturating_add(1))];
            assert!(matches!(
                cohere_messages_pt_v2(&messages),
                Err(CohereError::TooManyParts { limit }) if limit == MAX_MESSAGE_PARTS
            ));
        }
        Scenario::PartsAtLimit => {
            let messages = vec![parts_message(MAX_MESSAGE_PARTS)];
            drop(cohere_messages_pt_v2(&messages).unwrap());
        }
        Scenario::Preserved(source) => assert_preserved(source),
    }
}

#[test]
fn cohere_types_round_trip_as_json() {
    let result = cohere_messages_pt_v2(&parse_messages(
        r#"[{"role":"tool","tool_call_id":"call_1","content":"r"}]"#,
    ))
    .unwrap();
    assert!(matches!(result.message, CohereMessage::ToolResult(_)));
}
