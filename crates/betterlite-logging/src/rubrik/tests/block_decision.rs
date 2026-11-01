use betterlite_logging::rubrik::block_decision::{
    BlockedResponseResult, ModerationChoice, ModerationMessage, ModerationServiceResponse,
    ResponseBlockError, ReturnedToolCall, extract_prompt_refusal, extract_response_block,
};
use betterlite_logging::rubrik::tool_calls::{ChatCompletionMessageToolCall, Function};

fn call(id: &str) -> ChatCompletionMessageToolCall {
    ChatCompletionMessageToolCall {
        id: id.to_owned(),
        kind: "function".to_owned(),
        function: Function {
            name: "lookup".to_owned(),
            arguments: "{}".to_owned(),
        },
    }
}

fn response(content: Option<&str>, tool_call_ids: &[&str]) -> ModerationServiceResponse {
    ModerationServiceResponse {
        choices: vec![ModerationChoice {
            message: Some(ModerationMessage {
                content: content.map(str::to_owned),
                tool_calls: tool_call_ids
                    .iter()
                    .map(|id| ReturnedToolCall {
                        id: Some((*id).to_owned()),
                    })
                    .collect(),
            }),
        }],
    }
}

#[test]
fn prompt_refusal_returns_the_message_content_when_choices_are_present() {
    let response = response(Some("blocked for policy reasons"), &[]);
    assert_eq!(
        extract_prompt_refusal(&response),
        Some("blocked for policy reasons".to_owned())
    );
}

#[test]
fn prompt_refusal_defaults_when_content_is_missing() {
    let response = response(None, &[]);
    assert_eq!(
        extract_prompt_refusal(&response),
        Some("Request blocked by policy.".to_owned())
    );
}

#[test]
fn prompt_refusal_is_none_on_passthrough_with_empty_choices() {
    let response = ModerationServiceResponse { choices: vec![] };
    assert_eq!(extract_prompt_refusal(&response), None);
}

#[test]
fn response_block_is_none_when_text_matches_and_every_tool_call_came_back() {
    let sent = [call("call_1")];
    let response = response(Some("hello"), &["call_1"]);
    assert_eq!(extract_response_block(&response, &sent, "hello"), Ok(None));
}

#[test]
fn response_block_detects_text_replacement_when_content_was_swapped_wholesale() {
    let response = response(Some("Hello, blocked."), &[]);
    let result = extract_response_block(&response, &[], "Hello");
    assert_eq!(
        result,
        Ok(Some(BlockedResponseResult {
            explanation: "Hello, blocked.".to_owned(),
        }))
    );
}

#[test]
fn response_block_defaults_the_text_block_explanation_when_content_is_empty() {
    let response = response(Some(""), &[]);
    let result = extract_response_block(&response, &[], "Hello")
        .expect("ok")
        .expect("blocked");
    assert_eq!(result.explanation, "Response blocked by policy.");
}

#[test]
fn response_block_detects_a_missing_tool_call_by_cardinality() {
    let sent = [call("call_1"), call("call_2")];
    let response = response(Some(""), &["call_1"]);
    let result = extract_response_block(&response, &sent, "")
        .expect("ok")
        .expect("blocked");
    assert_eq!(result.explanation, "Tool call blocked by policy.");
}

#[test]
fn response_block_uses_counter_semantics_so_a_missing_duplicate_id_is_still_caught() {
    let sent = [call("call_1"), call("call_1")];
    let response = response(Some(""), &["call_1"]);
    let result = extract_response_block(&response, &sent, "").expect("ok");
    assert!(result.is_some());
}

#[test]
fn response_block_is_none_when_duplicate_ids_all_come_back() {
    let sent = [call("call_1"), call("call_1")];
    let response = response(Some(""), &["call_1", "call_1"]);
    assert_eq!(extract_response_block(&response, &sent, ""), Ok(None));
}

#[test]
fn response_block_extracts_the_appended_tail_as_the_explanation() {
    let sent = [call("call_1"), call("call_2")];
    let response = response(Some("hello\n\nblocked: missing a tool call"), &["call_1"]);
    let result = extract_response_block(&response, &sent, "hello")
        .expect("ok")
        .expect("blocked");
    assert_eq!(result.explanation, "blocked: missing a tool call");
}

#[test]
fn response_block_defaults_the_tool_block_explanation_when_the_appended_tail_is_empty() {
    let sent = [call("call_1"), call("call_2")];
    let response = response(Some("hello\n\n"), &["call_1"]);
    let result = extract_response_block(&response, &sent, "hello")
        .expect("ok")
        .expect("blocked");
    assert_eq!(result.explanation, "Tool call blocked by policy.");
}

#[test]
fn response_block_rejects_a_malformed_response_with_no_choices() {
    let response = ModerationServiceResponse { choices: vec![] };
    assert_eq!(
        extract_response_block(&response, &[], ""),
        Err(ResponseBlockError::MalformedResponse)
    );
}
