#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::Object;
use rstest::rstest;
use serde_json::{Value, json};

use super::error::OutputRewriteError;
use super::{
    OutputContext, apply_guardrail_responses_to_output, extract_output, process_output_response,
    write_tool_call_item, write_tool_call_rewrites_to_output,
};
use crate::guardrails::responses_translation::request_rewrite::tests::fake::FakeGuardrail;
use crate::guardrails::tool_call_identity::ToolCallShape;

type Processed = (Result<(), OutputRewriteError>, Value, Option<Value>);

async fn process(
    response: Value,
    guardrail: &FakeGuardrail,
    request_data: Option<Value>,
) -> Processed {
    let mut value: GuardrailValue = serde_json::from_value(response).unwrap();
    let mut stored: Option<Object> = request_data.map(|data| serde_json::from_value(data).unwrap());
    let context = OutputContext {
        guardrail_name: Some("guard"),
        user_api_key_dict: None,
    };
    let result =
        process_output_response(&mut value, guardrail, &context, None, stored.as_mut()).await;
    let stored_json = stored.map(|data| serde_json::to_value(data).unwrap());
    (result, serde_json::to_value(&value).unwrap(), stored_json)
}

fn shape(name: &str, arguments: &str) -> ToolCallShape {
    ToolCallShape {
        name: Some(name.to_owned()),
        arguments: arguments.to_owned(),
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn extract_output_collects_non_empty_texts_with_their_positions() {
    let items = [
        json!({"type": "message", "content": [
            {"type": "output_text", "text": "a"},
            {"type": "output_text", "text": ""},
            {"type": "refusal"},
            "stray",
            {"text": 5},
            {"type": "output_text", "text": "b"}
        ]}),
        json!({"type": "reasoning", "summary": []}),
        json!("bare string"),
        json!({"content": []}),
        json!({"content": "not a list"}),
        json!({"content": [{"text": "c"}]}),
    ];
    let extracted = extract_output(&items).unwrap();
    assert_eq!(extracted.texts, strings(&["a", "b", "c"]));
    assert_eq!(extracted.task_mappings, [(0, 0), (0, 5), (5, 0)]);
    assert!(extracted.tool_calls.is_empty());
}

#[test]
fn extract_output_turns_tool_call_items_into_chat_calls_without_text() {
    let items = [
        json!({"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "f", "arguments": "{\"a\":1}", "content": [{"text": "ignored"}]}),
        json!({"type": "custom_tool_call", "id": "ctc_1", "call_id": "call_2", "input": "raw"}),
    ];
    let extracted = extract_output(&items).unwrap();
    assert!(extracted.texts.is_empty());
    assert!(extracted.task_mappings.is_empty());
    let calls: Vec<(Value, Value, Value)> = extracted
        .tool_calls
        .iter()
        .map(|call| {
            (
                call["type"].clone(),
                call["function"].clone(),
                call["index"].clone(),
            )
        })
        .collect();
    assert_eq!(
        calls,
        [
            (
                json!("function"),
                json!({"name": "f", "arguments": "{\"a\":1}"}),
                json!(0)
            ),
            (
                json!("function"),
                json!({"name": "custom_tool", "arguments": "raw"}),
                json!(1)
            ),
        ]
    );
}

#[test]
fn extract_output_rejects_a_tool_call_item_with_non_text_arguments() {
    let items = [json!({"type": "function_call", "name": "f", "arguments": 5})];
    let error = extract_output(&items).unwrap_err();
    assert!(matches!(error, OutputRewriteError::ToolCallItem(_)));
}

#[test]
fn apply_output_writes_each_text_to_its_mapped_entry() {
    let mut response = json!({"output": [
        {"content": [{"text": "a"}, {"text": "b"}]},
        {"content": [{"text": "c"}]}
    ]});
    apply_guardrail_responses_to_output(&mut response, &strings(&["B", "C"]), &[(0, 1), (1, 0)])
        .unwrap();
    assert_eq!(
        response,
        json!({"output": [
            {"content": [{"text": "a"}, {"text": "B"}]},
            {"content": [{"text": "C"}]}
        ]})
    );
}

#[rstest]
#[case::output_index_past_the_end(json!({"output": [{"content": [{"text": "a"}]}]}), (5, 0))]
#[case::content_index_past_the_end(json!({"output": [{"content": [{"text": "a"}]}]}), (0, 9))]
#[case::item_is_not_an_object(json!({"output": ["bare"]}), (0, 0))]
#[case::content_is_not_a_list(json!({"output": [{"content": "text"}]}), (0, 0))]
#[case::entry_is_not_an_object(json!({"output": [{"content": ["text"]}]}), (0, 0))]
#[case::output_is_null(json!({"output": null}), (0, 0))]
#[case::response_is_not_an_object(json!("text"), (0, 0))]
fn apply_output_skips_a_text_it_cannot_place(
    #[case] original: Value,
    #[case] mapping: (usize, usize),
) {
    let mut response = original.clone();
    apply_guardrail_responses_to_output(&mut response, &strings(&["X"]), &[mapping]).unwrap();
    assert_eq!(response, original);
}

#[test]
fn apply_output_fails_on_a_text_without_a_mapping_after_the_earlier_writes() {
    let mut response = json!({"output": [{"content": [{"text": "a"}]}]});
    let error =
        apply_guardrail_responses_to_output(&mut response, &strings(&["A", "B"]), &[(0, 0)])
            .unwrap_err();
    assert!(matches!(
        error,
        OutputRewriteError::TooManyTexts {
            index: 1,
            scanned: 1
        }
    ));
    assert_eq!(response, json!({"output": [{"content": [{"text": "A"}]}]}));
}

#[rstest]
#[case::function_call_arguments(json!({"type": "function_call", "name": "f", "arguments": "{}"}), Some("g"), Some("{\"x\":1}"), json!({"type": "function_call", "name": "g", "arguments": "{\"x\":1}"}))]
#[case::custom_call_input(json!({"type": "custom_tool_call", "name": "f", "input": "a"}), None, Some("b"), json!({"type": "custom_tool_call", "name": "f", "input": "b"}))]
#[case::name_only(json!({"type": "function_call", "name": "f", "arguments": "{}"}), Some("g"), None, json!({"type": "function_call", "name": "g", "arguments": "{}"}))]
#[case::other_type_keeps_its_payload(json!({"type": "message", "arguments": "a"}), None, Some("b"), json!({"type": "message", "arguments": "a"}))]
#[case::untyped_item_keeps_its_payload(json!({"arguments": "a"}), None, Some("b"), json!({"arguments": "a"}))]
#[case::not_an_object(json!("text"), Some("g"), Some("b"), json!("text"))]
fn tool_rewrite_item_writes_the_name_and_the_payload_its_type_holds(
    #[case] original: Value,
    #[case] name: Option<&str>,
    #[case] payload: Option<&str>,
    #[case] expected: Value,
) {
    let mut item = original;
    write_tool_call_item(&mut item, name, payload);
    assert_eq!(item, expected);
}

fn tool_call_response() -> Value {
    json!({"output": [
        {"type": "function_call", "name": "f", "arguments": "{}"},
        {"type": "message", "content": [{"text": "t"}]},
        {"type": "custom_tool_call", "name": "g", "input": "x"}
    ]})
}

#[test]
fn tool_rewrite_output_rewrites_only_the_calls_that_changed() {
    let mut response = tool_call_response();
    let pre = [shape("f", "{}"), shape("g", "x")];
    let post = [shape("f", "{\"a\":1}"), shape("g", "x")];
    write_tool_call_rewrites_to_output(&mut response, &pre, &post);
    assert_eq!(
        response["output"][0],
        json!({"type": "function_call", "name": "f", "arguments": "{\"a\":1}"})
    );
    assert_eq!(
        response["output"][2],
        json!({"type": "custom_tool_call", "name": "g", "input": "x"})
    );
}

#[test]
fn tool_rewrite_output_renames_a_call_whose_name_changed() {
    let mut response = tool_call_response();
    let pre = [shape("f", "{}"), shape("g", "x")];
    let post = [shape("f", "{}"), shape("h", "y")];
    write_tool_call_rewrites_to_output(&mut response, &pre, &post);
    assert_eq!(response["output"][0]["name"], json!("f"));
    assert_eq!(
        response["output"][2],
        json!({"type": "custom_tool_call", "name": "h", "input": "y"})
    );
}

#[rstest]
#[case::one_call_returned(vec![shape("f", "{\"a\":1}")])]
#[case::an_extra_call_returned(vec![shape("f", "{\"a\":1}"), shape("g", "y"), shape("h", "z")])]
#[case::no_call_returned(vec![])]
fn tool_rewrite_output_ignores_a_count_that_differs(#[case] post: Vec<ToolCallShape>) {
    let mut response = tool_call_response();
    let pre = [shape("f", "{}"), shape("g", "x")];
    write_tool_call_rewrites_to_output(&mut response, &pre, &post);
    assert_eq!(response, tool_call_response());
}

#[test]
fn tool_rewrite_output_stops_at_the_shorter_scanned_list() {
    let mut response = tool_call_response();
    let post = [shape("f", "{\"a\":1}"), shape("g", "y")];
    write_tool_call_rewrites_to_output(&mut response, &[shape("f", "{}")], &post);
    assert_eq!(response["output"][0]["arguments"], json!("{\"a\":1}"));
    assert_eq!(response["output"][2]["input"], json!("x"));
}

#[rstest]
#[case::not_an_object(json!("text"))]
#[case::no_output(json!({"model": "m"}))]
#[case::null_output(json!({"output": null}))]
#[case::empty_output(json!({"output": []}))]
#[case::nothing_to_scan(json!({"output": [{"type": "reasoning"}, {"content": [{"text": ""}]}]}))]
#[tokio::test]
async fn process_output_makes_no_call_and_adds_no_response_key(#[case] response: Value) {
    let guardrail = FakeGuardrail::echoing();
    let (result, after, stored) = process(response.clone(), &guardrail, Some(json!({}))).await;
    result.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(after, response);
    assert_eq!(stored, Some(json!({})));
}

#[tokio::test]
async fn process_output_rejects_an_output_that_is_not_a_list() {
    let (result, _after, _stored) =
        process(json!({"output": "text"}), &FakeGuardrail::echoing(), None).await;
    assert!(matches!(
        result,
        Err(OutputRewriteError::FieldType { field: "output" })
    ));
}

#[tokio::test]
async fn process_output_sends_the_texts_the_model_and_the_response_as_request_data() {
    let response = json!({"model": "m", "output": [{"content": [{"text": "hi"}]}]});
    let guardrail = FakeGuardrail::echoing();
    let (result, after, stored) =
        process(response.clone(), &guardrail, Some(json!({"keep": 1}))).await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(calls[0].inputs.texts, Some(strings(&["hi"])));
    assert_eq!(calls[0].inputs.model.as_deref(), Some("m"));
    assert_eq!(calls[0].inputs.tool_calls, None);
    assert_eq!(
        stored,
        Some(json!({"keep": 1, "response": response.clone()}))
    );
    assert_eq!(after, response);
}

#[tokio::test]
async fn process_output_keeps_a_response_entry_the_request_data_already_has() {
    let response = json!({"output": [{"content": [{"text": "hi"}]}]});
    let (result, _after, stored) = process(
        response,
        &FakeGuardrail::echoing(),
        Some(json!({"response": "mine"})),
    )
    .await;
    result.unwrap();
    assert_eq!(stored, Some(json!({"response": "mine"})));
}

#[tokio::test]
async fn process_output_starts_from_empty_request_data_when_none_is_given() {
    let response = json!({"output": [{"content": [{"text": "hi"}]}]});
    let guardrail = FakeGuardrail::echoing();
    let (result, _after, stored) = process(response.clone(), &guardrail, None).await;
    result.unwrap();
    assert_eq!(stored, None);
    let sent = serde_json::to_value(&guardrail.calls()[0].request_data).unwrap();
    assert_eq!(sent, json!({"response": response}));
}

#[rstest]
#[case::empty(json!({"model": "", "output": [{"content": [{"text": "hi"}]}]}))]
#[case::not_text(json!({"model": 5, "output": [{"content": [{"text": "hi"}]}]}))]
#[tokio::test]
async fn process_output_leaves_the_model_out_unless_it_is_non_empty_text(#[case] response: Value) {
    let guardrail = FakeGuardrail::echoing();
    let (result, _after, _stored) = process(response, &guardrail, None).await;
    result.unwrap();
    assert_eq!(guardrail.calls()[0].inputs.model, None);
}

#[tokio::test]
async fn process_output_writes_texts_and_then_tool_call_rewrites() {
    let response = json!({"output": [
        {"content": [{"text": "hi"}]},
        {"type": "function_call", "id": "fc1", "call_id": "c1", "name": "f", "arguments": "{}"}
    ]});
    let reply = json!({"texts": ["HI"], "tool_calls": [
        {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{\"x\":1}"}, "index": 1}
    ]});
    let guardrail = FakeGuardrail::replying(reply);
    let (result, after, _stored) = process(response, &guardrail, None).await;
    result.unwrap();
    let sent = guardrail.calls()[0].inputs.tool_calls.clone().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(after["output"][0]["content"][0]["text"], json!("HI"));
    assert_eq!(after["output"][1]["arguments"], json!("{\"x\":1}"));
}

#[tokio::test]
async fn process_output_ignores_tool_calls_the_guardrail_returned_in_another_number() {
    let response = json!({"output": [
        {"content": [{"text": "hi"}]},
        {"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"}
    ]});
    let guardrail = FakeGuardrail::replying(json!({"texts": ["HI"], "tool_calls": []}));
    let (result, after, _stored) = process(response, &guardrail, None).await;
    result.unwrap();
    assert_eq!(after["output"][0]["content"][0]["text"], json!("HI"));
    assert_eq!(after["output"][1]["arguments"], json!("{}"));
}

#[tokio::test]
async fn process_output_keeps_the_earlier_texts_when_a_text_has_no_place() {
    let response = json!({"output": [
        {"content": [{"text": "hi"}]},
        {"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"}
    ]});
    let reply = json!({"texts": ["A", "B"], "tool_calls": [
        {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{\"x\":1}"}, "index": 1}
    ]});
    let (result, after, _stored) = process(response, &FakeGuardrail::replying(reply), None).await;
    assert!(matches!(
        result,
        Err(OutputRewriteError::TooManyTexts { index: 1, .. })
    ));
    assert_eq!(after["output"][0]["content"][0]["text"], json!("A"));
    assert_eq!(after["output"][1]["arguments"], json!("{}"));
}

#[tokio::test]
async fn process_output_passes_a_guardrail_failure_through() {
    let failure = GuardrailFailure::Other("blocked".to_owned());
    let response = json!({"output": [{"content": [{"text": "hi"}]}]});
    let guardrail = FakeGuardrail::failing(failure.clone());
    let (result, after, _stored) = process(response.clone(), &guardrail, None).await;
    assert!(matches!(
        result,
        Err(OutputRewriteError::Guardrail { source }) if *source == failure
    ));
    assert_eq!(after, response);
}

#[tokio::test]
async fn process_output_adds_the_user_key_metadata_to_the_request_data() {
    let mut response: GuardrailValue =
        serde_json::from_value(json!({"output": [{"content": [{"text": "hi"}]}]})).unwrap();
    let user: Object = serde_json::from_value(json!({"user_id": "u1"})).unwrap();
    let context = OutputContext {
        guardrail_name: None,
        user_api_key_dict: Some(&user),
    };
    let guardrail = FakeGuardrail::echoing();
    process_output_response(&mut response, &guardrail, &context, None, None)
        .await
        .unwrap();
    let sent = serde_json::to_value(&guardrail.calls()[0].request_data).unwrap();
    assert_eq!(
        sent["litellm_metadata"],
        json!({"user_api_key_user_id": "u1"})
    );
}
