#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_openai::search::records::SearchResponse;

use betterlite_websearch::call_transformation::outcome::{
    SearchFailure, SearchResult, search_outcome, search_outcome_text,
};
use betterlite_websearch::call_transformation::wire::{Response, Thinking};
use betterlite_websearch::call_transformation::{WebSearchTransformation, native};
use betterlite_websearch::records::{
    SearchFailed, SearchOutcome, SearchSucceeded, WebSearchToolResultErrorCode,
};

fn rate_limit_error() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"rate_limit","context":{"message":"slow down","model":null,"provider":null},"category":"vendor_rate_limit"}"#,
    )
    .unwrap()
}

fn bad_request_error() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"bad_request","message":"bad input","model":null,"provider":null}"#,
    )
    .unwrap()
}

fn unclassified_error() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"unclassified","message":"boom","model":null,"provider":null}"#,
    )
    .unwrap()
}

fn context_window_exceeded_error() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"context_window_exceeded","message":"too long","model":null,"provider":null}"#,
    )
    .unwrap()
}

fn content_policy_violation_error() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"content_policy_violation","message":"blocked","model":null,"provider":null}"#,
    )
    .unwrap()
}

fn unsupported_params_error() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"unsupported_params","message":"bad param","model":null,"provider":null}"#,
    )
    .unwrap()
}

#[test]
fn search_error_classifies_rate_limit_as_too_many_requests() {
    assert_eq!(
        native::search_error_code(&rate_limit_error()),
        WebSearchToolResultErrorCode::TooManyRequests
    );
}

#[test]
fn search_error_classifies_bad_request_as_invalid_tool_input() {
    assert_eq!(
        native::search_error_code(&bad_request_error()),
        WebSearchToolResultErrorCode::InvalidToolInput
    );
}

#[test]
fn search_error_classifies_other_errors_as_unavailable() {
    assert_eq!(
        native::search_error_code(&unclassified_error()),
        WebSearchToolResultErrorCode::Unavailable
    );
}

#[test]
fn search_error_classifies_bad_request_subclasses_as_invalid_tool_input() {
    assert_eq!(
        native::search_error_code(&context_window_exceeded_error()),
        WebSearchToolResultErrorCode::InvalidToolInput
    );
    assert_eq!(
        native::search_error_code(&content_policy_violation_error()),
        WebSearchToolResultErrorCode::InvalidToolInput
    );
    assert_eq!(
        native::search_error_code(&unsupported_params_error()),
        WebSearchToolResultErrorCode::InvalidToolInput
    );
}

#[test]
fn search_outcome_turns_failure_into_search_failed() {
    let failure = SearchFailure {
        class: rate_limit_error(),
        message: "slow down".to_owned(),
    };
    let outcome = search_outcome(SearchResult::Failed(failure));
    match outcome {
        SearchOutcome::Failed(failed) => {
            assert_eq!(
                failed.error_code,
                WebSearchToolResultErrorCode::TooManyRequests
            );
            assert_eq!(failed.message, "slow down");
        }
        SearchOutcome::Succeeded(_) => panic!("expected a failed outcome"),
    }
}

#[test]
fn search_outcome_turns_text_and_response_into_search_succeeded() {
    let response: SearchResponse = serde_json::from_str(r#"{"results":[]}"#).unwrap();
    let outcome = search_outcome(SearchResult::Succeeded {
        text: "hi".to_owned(),
        response: Some(response),
    });
    match outcome {
        SearchOutcome::Succeeded(succeeded) => {
            assert_eq!(succeeded.text, "hi");
            assert!(succeeded.response.is_some());
        }
        SearchOutcome::Failed(_) => panic!("expected a succeeded outcome"),
    }
}

#[test]
fn search_outcome_turns_text_without_response_into_search_succeeded() {
    let outcome = search_outcome(SearchResult::Succeeded {
        text: "hi".to_owned(),
        response: None,
    });
    match outcome {
        SearchOutcome::Succeeded(succeeded) => {
            assert_eq!(succeeded.text, "hi");
            assert!(succeeded.response.is_none());
        }
        SearchOutcome::Failed(_) => panic!("expected a succeeded outcome"),
    }
}

#[test]
fn search_outcome_turns_unexpected_results_into_search_succeeded_text() {
    let outcome = search_outcome(SearchResult::Unexpected("42".to_owned()));
    match outcome {
        SearchOutcome::Succeeded(succeeded) => {
            assert_eq!(succeeded.text, "42");
            assert!(succeeded.response.is_none());
        }
        SearchOutcome::Failed(_) => panic!("expected a succeeded outcome"),
    }
}

#[test]
fn search_outcome_text_extracts_text_from_a_succeeded_outcome() {
    let outcome = SearchOutcome::Succeeded(SearchSucceeded {
        text: "hi".to_owned(),
        response: None,
    });
    assert_eq!(search_outcome_text(&outcome), "hi");
}

#[test]
fn search_outcome_text_prefixes_the_message_of_a_failed_outcome() {
    let outcome = SearchOutcome::Failed(SearchFailed {
        error_code: WebSearchToolResultErrorCode::Unavailable,
        message: "down".to_owned(),
    });
    assert_eq!(search_outcome_text(&outcome), "Search failed: down");
}

#[test]
fn format_search_response_lists_title_url_and_snippet_per_result() {
    let response: SearchResponse = serde_json::from_str(
        r#"{"results":[{"title":"Rust","url":"https://rust-lang.org","snippet":"safe"}]}"#,
    )
    .unwrap();
    assert_eq!(
        native::format_search_response(&response).unwrap(),
        "Title: Rust\nURL: https://rust-lang.org\nSnippet: safe"
    );
}

#[test]
fn format_search_response_falls_back_to_the_response_text_when_empty() {
    let response: SearchResponse =
        serde_json::from_str(r#"{"results":[],"note":"empty"}"#).unwrap();
    assert_eq!(
        native::format_search_response(&response).unwrap(),
        "results=[] object='search' note='empty'"
    );
}

#[test]
fn transform_response_anthropic_builds_tool_use_and_tool_result_blocks() {
    let response: Response = serde_json::from_str(
        r#"{"content":[{"type":"tool_use","id":"call-1","name":"web_search","input":{"query":"rust"}}]}"#,
    )
    .unwrap();
    let (_, calls) =
        WebSearchTransformation::transform_request(&response, false, "anthropic").unwrap();
    let (assistant, user) = WebSearchTransformation::transform_response(
        &calls,
        &[String::from(
            "Title: Rust\nURL: https://rust-lang.org\nSnippet: safe",
        )],
        "anthropic",
    )
    .unwrap();
    let assistant_json = serde_json::to_value(assistant).unwrap();
    let user_json = serde_json::to_value(user).unwrap();
    assert_eq!(assistant_json["content"][0]["type"], "tool_use");
    assert_eq!(assistant_json["content"][0]["id"], "call-1");
    assert_eq!(user_json["content"][0]["tool_use_id"], "call-1");
    assert_eq!(
        user_json["content"][0]["content"],
        "Title: Rust\nURL: https://rust-lang.org\nSnippet: safe"
    );
}

#[test]
fn transform_response_anthropic_prepends_thinking_blocks_before_tool_use() {
    let response: Response = serde_json::from_str(
        r#"{"content":[{"type":"tool_use","id":"call-1","name":"web_search","input":{"query":"rust"}}]}"#,
    )
    .unwrap();
    let (_, calls) =
        WebSearchTransformation::transform_request(&response, false, "anthropic").unwrap();
    let (mut assistant, _) =
        WebSearchTransformation::transform_response(&calls, &[String::from("result")], "anthropic")
            .unwrap();
    assistant.prepend_thinking(&[Thinking::Thinking {
        thinking: "reasoning".to_owned(),
        signature: "signed".to_owned(),
    }]);
    let assistant_json = serde_json::to_value(assistant).unwrap();
    assert_eq!(assistant_json["content"][0]["type"], "thinking");
    assert_eq!(assistant_json["content"][0]["thinking"], "reasoning");
    assert_eq!(assistant_json["content"][1]["type"], "tool_use");
}

#[test]
fn transform_response_openai_builds_tool_calls_and_separate_tool_messages() {
    let response: Response = serde_json::from_str(
        r#"{"choices":[{"message":{"tool_calls":[{"id":"call-1","type":"function","function":{"name":"web_search","arguments":{"query":"rust"}}}]}}]}"#,
    )
    .unwrap();
    let (_, calls) =
        WebSearchTransformation::transform_request(&response, false, "openai").unwrap();
    let (assistant, tools) =
        WebSearchTransformation::transform_response(&calls, &[String::from("result")], "openai")
            .unwrap();
    let assistant_json = serde_json::to_value(assistant).unwrap();
    let tools_json = serde_json::to_value(tools).unwrap();
    assert_eq!(assistant_json["tool_calls"][0]["id"], "call-1");
    assert_eq!(tools_json[0]["tool_call_id"], "call-1");
    assert_eq!(tools_json[0]["content"], "result");
}

#[test]
fn build_web_search_tool_result_block_carries_snippet_alongside_empty_encrypted_content() {
    let response: SearchResponse = serde_json::from_str(
        r#"{"results":[{"title":"Rust","url":"https://rust-lang.org","snippet":"safe","date":"today"}]}"#,
    )
    .unwrap();
    let block = serde_json::to_value(native::result_block("call-1", Some(&response))).unwrap();
    assert_eq!(block["type"], "web_search_tool_result");
    assert_eq!(block["tool_use_id"], "call-1");
    assert_eq!(block["content"][0]["url"], "https://rust-lang.org");
    assert_eq!(block["content"][0]["title"], "Rust");
    assert_eq!(block["content"][0]["page_age"], "today");
    assert_eq!(block["content"][0]["encrypted_content"], "");
    assert_eq!(block["content"][0]["snippet"], "safe");
}

#[test]
fn build_web_search_tool_result_block_emits_an_empty_content_list_without_results() {
    let response: SearchResponse = serde_json::from_str(r#"{"results":[]}"#).unwrap();
    let block = serde_json::to_value(native::result_block("call-1", Some(&response))).unwrap();
    assert_eq!(block["content"].as_array().unwrap().len(), 0);
}

#[test]
fn build_web_search_tool_result_error_block_carries_the_error_code() {
    let block = serde_json::to_value(native::result_error_block(
        "call-1",
        WebSearchToolResultErrorCode::TooManyRequests,
    ))
    .unwrap();
    assert_eq!(block["type"], "web_search_tool_result");
    assert_eq!(block["content"]["type"], "web_search_tool_result_error");
    assert_eq!(block["content"]["error_code"], "too_many_requests");
}

#[test]
fn build_web_search_outcome_block_dispatches_succeeded_to_the_result_block() {
    let outcome = SearchOutcome::Succeeded(SearchSucceeded {
        text: "hi".to_owned(),
        response: None,
    });
    let block = serde_json::to_value(native::outcome_block("call-1", &outcome)).unwrap();
    assert_eq!(block["content"].as_array().unwrap().len(), 0);
}

#[test]
fn build_web_search_outcome_block_dispatches_failed_to_the_error_block() {
    let outcome = SearchOutcome::Failed(SearchFailed {
        error_code: WebSearchToolResultErrorCode::Unavailable,
        message: "down".to_owned(),
    });
    let block = serde_json::to_value(native::outcome_block("call-1", &outcome)).unwrap();
    assert_eq!(block["content"]["error_code"], "unavailable");
}
