#![cfg(test)]

use std::future::{Future, ready};

use rstest::rstest;
use tokio::runtime::Builder;

use crate::arguments::json::Json;
use crate::gemini_tool_invokes::error::GeminiToolInvokesError;
use crate::gemini_tool_invokes::invoke::convert_tool_calls_to_gemini;
use crate::gemini_tool_invokes::limits::MAX_TOOL_CALLS;
use crate::gemini_tool_invokes::signatures::DUMMY_THOUGHT_SIGNATURE;
use crate::gemini_tool_results::convert::convert_tool_result_to_gemini;
use crate::gemini_tool_results::error::GeminiToolResultsError;
use crate::gemini_tool_results::types::LastToolCallMessage;
use crate::media::inline::RemoteMediaFetcher;
use crate::media_fetch::error::ImageFetchError;
use crate::message_content::Message;

struct RefusingFetch;

impl RemoteMediaFetcher for RefusingFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

struct InvokeCase {
    message: &'static str,
    model: Option<&'static str>,
    forward: bool,
    expected: &'static str,
}

struct ResultCase {
    message: &'static str,
    last: Option<&'static str>,
    forward: bool,
    expected: &'static str,
}

enum Failure {
    Arguments,
    MissingName,
    Field(&'static str),
}

enum Scenario {
    Invoke(InvokeCase),
    InvokeFailure(&'static str, Failure),
    Result(ResultCase),
    ResultFailure(ResultCase, Failure),
    DummySignature,
    SignaturePassedThrough,
    TooManyCalls,
    MaxCalls,
}

fn parse_message(text: &str) -> Message {
    serde_json::from_str(text).unwrap()
}

fn parsed_json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn assert_invoke(case: &InvokeCase) {
    let message = parse_message(case.message);
    let parts = convert_tool_calls_to_gemini(&message, case.model, case.forward).unwrap();
    assert_eq!(
        serde_json::to_value(&parts).unwrap(),
        serde_json::to_value(parsed_json(case.expected)).unwrap()
    );
}

fn assert_result(case: &ResultCase) {
    let message = parse_message(case.message);
    let last = case
        .last
        .map(|text| serde_json::from_str::<LastToolCallMessage>(text).unwrap());
    let runtime = Builder::new_current_thread().build().unwrap();
    let result = runtime
        .block_on(convert_tool_result_to_gemini(
            &message,
            last.as_ref(),
            case.forward,
            &RefusingFetch,
        ))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        serde_json::to_value(parsed_json(case.expected)).unwrap()
    );
}

fn assert_invoke_failure(source: &str, failure: &Failure) {
    let message = parse_message(source);
    let error =
        convert_tool_calls_to_gemini(&message, Some("gemini-3-pro-preview"), false).unwrap_err();
    match failure {
        Failure::Arguments => assert!(
            matches!(error, GeminiToolInvokesError::Arguments { .. }),
            "unexpected {error}"
        ),
        Failure::Field(path) => {
            assert!(
                matches!(error, GeminiToolInvokesError::Fields { .. }),
                "unexpected {error}"
            );
            assert!(
                error.to_string().contains(path),
                "{error} does not name {path}"
            );
        }
        Failure::MissingName => panic!("a missing name is a tool result failure"),
    }
}

fn assert_result_failure(case: &ResultCase, failure: &Failure) {
    let message = parse_message(case.message);
    let last = case
        .last
        .map(|text| serde_json::from_str::<LastToolCallMessage>(text).unwrap());
    let runtime = Builder::new_current_thread().build().unwrap();
    let error = runtime
        .block_on(convert_tool_result_to_gemini(
            &message,
            last.as_ref(),
            case.forward,
            &RefusingFetch,
        ))
        .unwrap_err();
    assert!(
        matches!(
            (&error, failure),
            (GeminiToolResultsError::MissingName, Failure::MissingName)
        ),
        "unexpected {error}"
    );
}

#[rstest]
#[case::invoke_parts_carry_signatures(Scenario::Invoke(InvokeCase {
    message: r#"{"role":"assistant","content":"hi","tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"},"provider_specific_fields":{"thought_signature":"sig-one"}}]}"#,
    model: None,
    forward: true,
    expected: r#"[{"function_call":{"name":"get_weather","args":{"city":"Paris"},"id":"call_1"},"thoughtSignature":"sig-one"}]"#,
}))]
#[case::arguments_forward_only_when_asked(Scenario::Invoke(InvokeCase {
    message: r#"{"role":"assistant","content":"hi","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":""}}]}"#,
    model: None,
    forward: false,
    expected: r#"[{"function_call":{"name":"f","args":{"type":"object"}}}]"#,
}))]
#[case::tool_result_carries_the_recovered_name(Scenario::Result(ResultCase {
    message: r#"{"role":"tool","tool_call_id":"call_1","content":"{\"temperature\":22}"}"#,
    last: Some(r#"{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}]}"#),
    forward: true,
    expected: r#"{"function_response":{"name":"get_weather","response":{"temperature":22},"id":"call_1"}}"#,
}))]
#[case::tool_result_without_forwarding_omits_the_id(Scenario::Result(ResultCase {
    message: r#"{"role":"tool","tool_call_id":"call_1","content":"sunny"}"#,
    last: Some(r#"{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}]}"#),
    forward: false,
    expected: r#"{"function_response":{"name":"get_weather","response":{"content":"sunny"}}}"#,
}))]
#[case::malformed_arguments_fail(Scenario::InvokeFailure(
    r#"{"role":"assistant","content":"hi","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{"}}]}"#,
    Failure::Arguments,
))]
#[case::malformed_signatures_fail(Scenario::InvokeFailure(
    r#"{"role":"assistant","content":"hi","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}","provider_specific_fields":{"thought_signature":5}}}]}"#,
    Failure::Field("thought_signature"),
))]
#[case::too_many_tool_calls_fail(Scenario::TooManyCalls)]
#[case::the_tool_call_limit_is_accepted(Scenario::MaxCalls)]
#[case::gemini_three_signs_the_first_call(Scenario::DummySignature)]
#[case::existing_signatures_are_not_regenerated(Scenario::SignaturePassedThrough)]
#[case::tool_results_need_a_matching_call(Scenario::ResultFailure(ResultCase {
    message: r#"{"role":"tool","tool_call_id":"call_9","content":"x"}"#,
    last: None,
    forward: false,
    expected: "",
}, Failure::MissingName))]
fn other_prompts_provider_message_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Invoke(case) => assert_invoke(&case),
        Scenario::InvokeFailure(message, failure) => assert_invoke_failure(message, &failure),
        Scenario::Result(case) => assert_result(&case),
        Scenario::ResultFailure(case, failure) => assert_result_failure(&case, &failure),
        Scenario::DummySignature => {
            let message = parse_message(
                r#"{"role":"assistant","content":"hi","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]}"#,
            );
            let parts = convert_tool_calls_to_gemini(&message, Some("gemini-3-pro-preview"), false)
                .unwrap();
            let signature = parts
                .first()
                .and_then(|part| part.thought_signature.as_deref());
            assert_eq!(signature, Some(DUMMY_THOUGHT_SIGNATURE));
        }
        Scenario::SignaturePassedThrough => {
            let message = parse_message(
                r#"{"role":"assistant","content":"hi","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"},"provider_specific_fields":{"thought_signature":"sig-kept"}}]}"#,
            );
            let parts = convert_tool_calls_to_gemini(&message, Some("gemini-3-pro-preview"), false)
                .unwrap();
            let signature = parts
                .first()
                .and_then(|part| part.thought_signature.as_deref());
            assert_eq!(signature, Some("sig-kept"));
        }
        Scenario::TooManyCalls => {
            let tools = tool_calls(MAX_TOOL_CALLS.saturating_add(1));
            let message = message_with_tools(&tools);
            assert!(matches!(
                convert_tool_calls_to_gemini(&message, None, false),
                Err(GeminiToolInvokesError::TooManyToolCalls { limit }) if limit == MAX_TOOL_CALLS
            ));
        }
        Scenario::MaxCalls => {
            let tools = tool_calls(MAX_TOOL_CALLS);
            let message = message_with_tools(&tools);
            let parts = convert_tool_calls_to_gemini(&message, None, false).unwrap();
            assert_eq!(parts.len(), MAX_TOOL_CALLS);
        }
    }
}

fn tool_calls(count: usize) -> String {
    let call = r#"{"id":"call","type":"function","function":{"name":"f","arguments":"{}"}}"#;
    vec![call; count].join(",")
}

fn message_with_tools(tools: &str) -> Message {
    parse_message(&format!(
        r#"{{"role":"assistant","content":"hi","tool_calls":[{tools}]}}"#
    ))
}
