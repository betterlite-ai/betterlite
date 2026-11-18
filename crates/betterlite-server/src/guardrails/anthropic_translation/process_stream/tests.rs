#![cfg(test)]

use std::future::Future;
use std::str;
use std::sync::Mutex;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use serde_json::{Value, json};

use super::error::ProcessStreamError;
use super::process_output_streaming_response;
use crate::guardrails::anthropic_translation::sse_events::SseChunk;

const START: &str = "event: message_start\ndata: {\"message\": {\"id\": \"msg_1\", \"model\": \"claude-test\", \"role\": \"assistant\", \"usage\": {\"input_tokens\": 4, \"output_tokens\": 1}}, \"type\": \"message_start\"}\n\n";
const TEXT_1: &str = "event: content_block_delta\ndata: {\"delta\": {\"text\": \"Hello\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\n";
const TEXT_2: &str = "event: content_block_delta\ndata: {\"delta\": {\"text\": \" there\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\n";
const TOOL_START: &str = "event: content_block_start\ndata: {\"content_block\": {\"id\": \"toolu_1\", \"input\": {}, \"name\": \"get_weather\", \"type\": \"tool_use\"}, \"index\": 0, \"type\": \"content_block_start\"}\n\n";
const TOOL_ARGS: &str = "event: content_block_delta\ndata: {\"delta\": {\"partial_json\": \"{\\\"city\\\": \\\"Paris\\\"}\", \"type\": \"input_json_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\n";
const STOP_TEXT: &str = "event: message_delta\ndata: {\"delta\": {\"stop_reason\": \"end_turn\"}, \"type\": \"message_delta\", \"usage\": {\"output_tokens\": 9}}\n\n";
const STOP_TOOL: &str = "event: message_delta\ndata: {\"delta\": {\"stop_reason\": \"tool_use\"}, \"type\": \"message_delta\", \"usage\": {\"output_tokens\": 7}}\n\n";
const USAGE_ONLY_DELTA: &str = "event: message_delta\ndata: {\"delta\": {}, \"type\": \"message_delta\", \"usage\": {\"output_tokens\": 9}}\n\n";

#[derive(Clone, Debug)]
struct Recorded {
    inputs: GuardrailApiInputs,
    input_type: GuardrailInputType,
    request_data: Object,
}

struct FakeGuardrail {
    reply: Option<GuardrailApiInputs>,
    failure: Option<GuardrailFailure>,
    calls: Mutex<Vec<Recorded>>,
}

impl FakeGuardrail {
    fn echoing() -> Self {
        Self {
            reply: None,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn replying(reply: Value) -> Self {
        Self {
            reply: Some(serde_json::from_value(reply).unwrap()),
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(failure: GuardrailFailure) -> Self {
        Self {
            reply: None,
            failure: Some(failure),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Recorded> {
        self.calls.lock().unwrap().clone()
    }
}

impl GuardrailContract for FakeGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut Object,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.calls.lock().unwrap().push(Recorded {
            inputs: inputs.clone(),
            input_type,
            request_data: request_data.clone(),
        });
        let outcome = match (&self.failure, &self.reply) {
            (Some(failure), _) => Err(failure.clone()),
            (None, Some(reply)) => Ok(reply.clone()),
            (None, None) => Ok(inputs),
        };
        async move { outcome }
    }
}

type Processed = (Result<(), ProcessStreamError>, Vec<SseChunk>, Option<Value>);

fn bytes(parts: &[&str]) -> SseChunk {
    SseChunk::Bytes(parts.concat().into_bytes())
}

fn event(value: Value) -> SseChunk {
    SseChunk::Event(value.as_object().unwrap().clone())
}

fn text_of(chunks: &[SseChunk]) -> String {
    let mut text = String::new();
    for chunk in chunks {
        if let SseChunk::Bytes(bytes) = chunk {
            text.push_str(str::from_utf8(bytes).unwrap());
        }
    }
    text
}

fn modify_response(message: &str) -> GuardrailFailure {
    GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: message.to_owned(),
        model: String::new(),
        request_data: Object::new(),
        guardrail_name: Some("guard".to_owned()),
        detection_info: Object::new(),
        original_response: None,
    }))
}

async fn process(
    chunks: Vec<SseChunk>,
    guardrail: &FakeGuardrail,
    request_data: Option<Value>,
    user_api_key_dict: Option<Value>,
    deliver: bool,
) -> Processed {
    let mut stored: Option<Object> = request_data.map(|data| serde_json::from_value(data).unwrap());
    let user: Option<Object> = user_api_key_dict.map(|user| serde_json::from_value(user).unwrap());
    let mut responses = chunks;
    let result = process_output_streaming_response(
        &mut responses,
        guardrail,
        Some("guard"),
        None,
        user.as_ref(),
        stored.as_mut(),
        deliver,
    )
    .await;
    let stored_json = stored.map(|data| serde_json::to_value(data).unwrap());
    (result, responses, stored_json)
}

fn inputs_json(recorded: &Recorded) -> Value {
    serde_json::to_value(&recorded.inputs).unwrap()
}

#[tokio::test]
async fn ended_passes_the_assembled_text_and_response() {
    let guardrail = FakeGuardrail::echoing();
    let stream = vec![bytes(&[START, TEXT_1, TEXT_2, STOP_TEXT])];
    let (result, _, stored) = process(stream, &guardrail, Some(json!({})), None, false).await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(calls[0].inputs.texts, Some(vec!["Hello there".to_owned()]));
    assert_eq!(calls[0].inputs.tool_calls, None);
    assert_eq!(calls[0].inputs.model, None);
    assert_eq!(calls[0].inputs.images, None);
    let data = stored.unwrap();
    assert_eq!(
        data["response"]["choices"][0]["message"]["content"],
        "Hello there"
    );
    assert_eq!(data["response"]["usage"]["prompt_tokens"], 4);
}

#[tokio::test]
async fn ended_passes_the_streamed_tool_call() {
    let guardrail = FakeGuardrail::echoing();
    let stream = vec![bytes(&[START, TOOL_START, TOOL_ARGS, STOP_TOOL])];
    let (result, _, _) = process(stream, &guardrail, None, None, false).await;
    result.unwrap();
    let calls = guardrail.calls();
    let inputs = inputs_json(&calls[0]);
    assert_eq!(inputs["texts"], Value::Null);
    assert_eq!(inputs["tool_calls"][0]["function"]["name"], "get_weather");
    assert_eq!(
        inputs["tool_calls"][0]["function"]["arguments"],
        "{\"city\": \"Paris\"}"
    );
}

#[tokio::test]
async fn ended_skips_the_guardrail_when_the_response_has_no_choices() {
    let guardrail = FakeGuardrail::echoing();
    let stream = vec![bytes(&[
        "event: message_delta\ndata: not-json\ndata: {\"delta\": {\"stop_reason\": \"end_turn\"}, \"type\": \"message_delta\"}\n\n",
    ])];
    let (result, _, stored) = process(stream, &guardrail, None, None, false).await;
    result.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(stored, None);
}

#[tokio::test]
async fn ended_reports_a_modify_response_with_the_assembled_response() {
    let guardrail = FakeGuardrail::failing(modify_response("blocked"));
    let stream = vec![bytes(&[START, TEXT_1, STOP_TEXT])];
    let (result, _, _) = process(stream, &guardrail, None, None, false).await;
    let ProcessStreamError::Guardrail(GuardrailFailure::ModifyResponse(response)) =
        result.unwrap_err()
    else {
        panic!("expected a modify response");
    };
    let original = serde_json::to_value(response.original_response.unwrap()).unwrap();
    assert_eq!(original["choices"][0]["message"]["content"], "Hello");
}

#[tokio::test]
async fn ended_keeps_a_modify_response_that_has_an_original_response() {
    let mut failure = modify_response("blocked");
    if let GuardrailFailure::ModifyResponse(response) = &mut failure {
        response.original_response = Some(GuardrailValue::Text("kept".to_owned()));
    }
    let guardrail = FakeGuardrail::failing(failure);
    let stream = vec![bytes(&[START, TEXT_1, STOP_TEXT])];
    let (result, _, _) = process(stream, &guardrail, None, None, false).await;
    let ProcessStreamError::Guardrail(GuardrailFailure::ModifyResponse(response)) =
        result.unwrap_err()
    else {
        panic!("expected a modify response");
    };
    assert_eq!(
        response.original_response,
        Some(GuardrailValue::Text("kept".to_owned()))
    );
}

#[tokio::test]
async fn ended_returns_another_failure_as_it_is() {
    let guardrail = FakeGuardrail::failing(GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    });
    let stream = vec![bytes(&[START, TEXT_1, STOP_TEXT])];
    let (result, _, _) = process(stream, &guardrail, None, None, false).await;
    let ProcessStreamError::Guardrail(GuardrailFailure::Http { status, detail }) =
        result.unwrap_err()
    else {
        panic!("expected the http failure");
    };
    assert_eq!(status, 400);
    assert_eq!(detail, "blocked");
}

#[tokio::test]
async fn ended_writes_the_text_rewrite() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["Rewrite"]}));
    let stream = vec![bytes(&[START, TEXT_1, TEXT_2, STOP_TEXT])];
    let (result, responses, _) = process(stream, &guardrail, None, None, true).await;
    result.unwrap();
    let rewritten = text_of(&responses);
    assert!(rewritten.contains(r#""text": "Rewrite""#));
    assert!(rewritten.contains(r#""text": """#));
    assert!(!rewritten.contains(r#""text": "Hello""#));
}

#[tokio::test]
async fn ended_writes_nothing_without_delivery() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["Rewrite"]}));
    let stream = vec![bytes(&[START, TEXT_1, TEXT_2, STOP_TEXT])];
    let (result, responses, _) = process(stream, &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(
        text_of(&responses),
        [START, TEXT_1, TEXT_2, STOP_TEXT].concat()
    );
}

#[tokio::test]
async fn ended_rewrites_a_returned_tool_call() {
    let guardrail = FakeGuardrail::replying(
        json!({"tool_calls": [{"function": {"name": "other_tool", "arguments": "{\"x\": 1}"}}]}),
    );
    let stream = vec![bytes(&[START, TOOL_START, TOOL_ARGS, STOP_TOOL])];
    let (result, responses, _) = process(stream, &guardrail, None, None, true).await;
    result.unwrap();
    let rewritten = text_of(&responses);
    assert!(rewritten.contains(r#""name": "other_tool""#));
    assert!(!rewritten.contains("get_weather"));
    assert!(rewritten.contains(r#""partial_json": "{\"x\": 1}""#));
}

#[tokio::test]
async fn ended_ignores_a_returned_tool_list_of_another_length() {
    let guardrail = FakeGuardrail::replying(json!({"tool_calls": [
        {"function": {"name": "one", "arguments": "{}"}},
        {"function": {"name": "two", "arguments": "{}"}}
    ]}));
    let stream = vec![bytes(&[START, TOOL_START, TOOL_ARGS, STOP_TOOL])];
    let (result, responses, _) = process(stream, &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(
        text_of(&responses),
        [START, TOOL_START, TOOL_ARGS, STOP_TOOL].concat()
    );
}

#[tokio::test]
async fn ended_writes_nothing_when_the_text_is_unchanged() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["Hello there"]}));
    let stream = vec![bytes(&[START, TEXT_1, TEXT_2, STOP_TEXT])];
    let (result, responses, _) = process(stream, &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(
        text_of(&responses),
        [START, TEXT_1, TEXT_2, STOP_TEXT].concat()
    );
}

#[tokio::test]
async fn ended_skips_the_text_rewrite_without_content() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["Rewrite"]}));
    let stream = vec![bytes(&[START, TOOL_START, TOOL_ARGS, STOP_TOOL])];
    let (result, responses, _) = process(stream, &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(
        text_of(&responses),
        [START, TOOL_START, TOOL_ARGS, STOP_TOOL].concat()
    );
}

#[tokio::test]
async fn unended_sends_the_string_so_far_and_the_chunks() {
    let guardrail = FakeGuardrail::echoing();
    let chunk = event(
        json!({"delta": {"text": "curious", "type": "text_delta"}, "type": "content_block_delta"}),
    );
    let stream = vec![bytes(&[START]), chunk, SseChunk::Other];
    let key_dict = json!({"user_id": "u1"});
    let (result, _, stored) =
        process(stream, &guardrail, Some(json!({})), Some(key_dict), false).await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls[0].inputs.texts, Some(vec!["curious".to_owned()]));
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    let data = stored.unwrap();
    let responses = data["responses"].as_array().unwrap();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0], START);
    assert!(responses[1].is_object());
    assert!(responses[2].is_null());
    assert_eq!(data["litellm_metadata"]["user_api_key_user_id"], "u1");
}

#[tokio::test]
async fn unended_replaces_invalid_utf8_in_the_chunk_values() {
    let guardrail = FakeGuardrail::echoing();
    let stream = vec![SseChunk::Bytes(b"bad \xff end".to_vec())];
    let (result, _, _) = process(stream, &guardrail, None, None, false).await;
    result.unwrap();
    let recorded = serde_json::to_value(&guardrail.calls()[0].request_data).unwrap();
    assert_eq!(recorded["responses"][0], "bad \u{FFFD} end");
}

#[tokio::test]
async fn unended_sends_an_empty_string_without_text_deltas() {
    let guardrail = FakeGuardrail::echoing();
    let stream = vec![bytes(&[START])];
    let (result, _, _) = process(stream, &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(guardrail.calls()[0].inputs.texts, Some(vec![String::new()]));
}

#[tokio::test]
async fn unended_reports_a_modify_response_with_the_usage_response() {
    let guardrail = FakeGuardrail::failing(modify_response("blocked"));
    let stream = vec![bytes(&[START, TEXT_1, USAGE_ONLY_DELTA])];
    let (result, _, _) = process(stream, &guardrail, None, None, false).await;
    let ProcessStreamError::Guardrail(GuardrailFailure::ModifyResponse(response)) =
        result.unwrap_err()
    else {
        panic!("expected a modify response");
    };
    let original = serde_json::to_value(response.original_response.unwrap()).unwrap();
    assert!(original["usage"]["completion_tokens"].is_number());
}

#[tokio::test]
async fn unended_leaves_the_original_response_empty_without_usage() {
    let guardrail = FakeGuardrail::failing(modify_response("blocked"));
    let stream = vec![event(
        json!({"delta": {"text": "hi", "type": "text_delta"}, "type": "content_block_delta"}),
    )];
    let (result, _, _) = process(stream, &guardrail, None, None, false).await;
    let ProcessStreamError::Guardrail(GuardrailFailure::ModifyResponse(response)) =
        result.unwrap_err()
    else {
        panic!("expected a modify response");
    };
    assert_eq!(response.original_response, None);
}

#[tokio::test]
async fn unended_writes_a_changed_text() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["changed"]}));
    let stream = vec![bytes(&[START, TEXT_1])];
    let (result, responses, _) = process(stream, &guardrail, None, None, true).await;
    result.unwrap();
    assert!(text_of(&responses).contains(r#""text": "changed""#));
}

#[tokio::test]
async fn unended_writes_nothing_for_the_same_text() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["Hello"]}));
    let stream = vec![bytes(&[START, TEXT_1])];
    let (result, responses, _) = process(stream, &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(text_of(&responses), [START, TEXT_1].concat());
}

#[tokio::test]
async fn unended_writes_the_first_of_several_texts() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["first", "second"]}));
    let stream = vec![bytes(&[START, TEXT_1])];
    let (result, responses, _) = process(stream, &guardrail, None, None, true).await;
    result.unwrap();
    let rewritten = text_of(&responses);
    assert!(rewritten.contains(r#""text": "first""#));
    assert!(!rewritten.contains("second"));
}

#[tokio::test]
async fn unended_writes_nothing_for_empty_texts() {
    let guardrail = FakeGuardrail::replying(json!({"texts": []}));
    let stream = vec![bytes(&[START, TEXT_1])];
    let (result, responses, _) = process(stream, &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(text_of(&responses), [START, TEXT_1].concat());
}

#[tokio::test]
async fn unended_writes_the_first_when_more_texts_follow() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["Hello there", "extra"]}));
    let stream = vec![bytes(&[START, TEXT_1, TEXT_2])];
    let (result, responses, _) = process(stream, &guardrail, None, None, true).await;
    result.unwrap();
    let rewritten = text_of(&responses);
    assert!(rewritten.contains(r#""text": "Hello there""#));
    assert!(!rewritten.contains(r#""text": " there""#));
}

#[tokio::test]
async fn unended_writes_nothing_without_delivery() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["changed"]}));
    let stream = vec![bytes(&[START, TEXT_1])];
    let (result, responses, _) = process(stream, &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(text_of(&responses), [START, TEXT_1].concat());
}

#[tokio::test]
async fn unended_skips_metadata_without_a_user_key() {
    let guardrail = FakeGuardrail::echoing();
    let stream = vec![bytes(&[START])];
    let (result, _, stored) = process(stream, &guardrail, Some(json!({})), None, false).await;
    result.unwrap();
    let data = stored.unwrap();
    assert!(data.get("responses").is_some());
    assert!(data.get("litellm_metadata").is_none());
}

#[tokio::test]
async fn unended_keeps_an_existing_key_in_the_request_data() {
    let guardrail = FakeGuardrail::echoing();
    let stream = vec![bytes(&[START])];
    let given = json!({"responses": null, "keep": 1});
    let (result, _, stored) = process(stream, &guardrail, Some(given), None, false).await;
    result.unwrap();
    let data = stored.unwrap();
    assert_eq!(data["responses"], Value::Null);
    assert_eq!(data["keep"], json!(1));
}

#[tokio::test]
async fn unended_keeps_an_existing_litellm_metadata() {
    let guardrail = FakeGuardrail::echoing();
    let stream = vec![bytes(&[START])];
    let given = json!({"litellm_metadata": {"kept": true}});
    let (result, _, stored) = process(
        stream,
        &guardrail,
        Some(given),
        Some(json!({"user_id": "u1"})),
        false,
    )
    .await;
    result.unwrap();
    let data = stored.unwrap();
    assert_eq!(data["litellm_metadata"], json!({"kept": true}));
}

#[tokio::test]
async fn unended_names_an_unknown_guardrail_on_an_undeliverable_rewrite() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["changed"]}));
    let mut responses = vec![bytes(&[START])];
    let result =
        process_output_streaming_response(&mut responses, &guardrail, None, None, None, None, true)
            .await;
    let ProcessStreamError::Undeliverable(failure) = result.unwrap_err() else {
        panic!("expected an undeliverable rewrite");
    };
    assert_eq!(failure.guardrail_name, "unknown");
}
