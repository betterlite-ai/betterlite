#![cfg(test)]

use std::future::Future;
use std::sync::Mutex;

use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use rstest::rstest;
use serde_json::{Value, json};

use super::error::StreamOutputError;
use super::process_output_streaming_response;
use crate::guardrails::responses_translation::output_rewrite::error::OutputRewriteError;

#[derive(Clone, Debug)]
struct Recorded {
    inputs: GuardrailApiInputs,
    input_type: GuardrailInputType,
    request_data: Object,
}

// Answers with the scripted inputs, or echoes what it was sent when nothing is scripted
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

type Processed = (Result<(), StreamOutputError>, Value, Option<Value>);

async fn process(
    events: Value,
    guardrail: &FakeGuardrail,
    request_data: Option<Value>,
    user_api_key_dict: Option<Value>,
    deliver: bool,
) -> Processed {
    let mut events: Vec<Value> = serde_json::from_value(events).unwrap();
    let mut stored: Option<Object> = request_data.map(|data| serde_json::from_value(data).unwrap());
    let user: Option<Object> = user_api_key_dict.map(|user| serde_json::from_value(user).unwrap());
    let result = process_output_streaming_response(
        &mut events,
        guardrail,
        Some("guard"),
        None,
        user.as_ref(),
        stored.as_mut(),
        deliver,
    )
    .await;
    let events = serde_json::to_value(events).unwrap();
    let stored = stored.map(|data| serde_json::to_value(data).unwrap());
    (result, events, stored)
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn delta(item_id: &str, output_index: u32, content_index: u32, value: &str) -> Value {
    json!({
        "type": "response.output_text.delta",
        "item_id": item_id,
        "output_index": output_index,
        "content_index": content_index,
        "delta": value,
    })
}

fn message_item() -> Value {
    json!({"type": "message", "id": "m1", "content": [{"type": "output_text", "text": "hello"}]})
}

fn text_stream() -> Value {
    json!([
        delta("m1", 0, 0, "he"),
        delta("m1", 0, 0, "llo"),
        {"type": "response.completed", "response": {"model": "gpt-4o", "output": [message_item()]}}
    ])
}

fn tool_call_stream() -> Value {
    json!([
        {"type": "response.output_item.added", "output_index": 1, "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": ""}},
        {"type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 1, "delta": "{\"q\""},
        {"type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 1, "delta": ":1}"},
        {"type": "response.function_call_arguments.done", "item_id": "fc_1", "output_index": 1, "arguments": "{\"q\":1}"},
        {"type": "response.output_item.done", "output_index": 1, "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": "{\"q\":1}"}},
        {"type": "response.completed", "response": {"output": [
            {"type": "message", "id": "m0", "content": [{"type": "output_text", "text": "hello"}]},
            {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": "{\"q\":1}"}
        ]}}
    ])
}

fn function_call_item(arguments: &str) -> Value {
    json!({"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": arguments})
}

#[tokio::test]
async fn terminal_scans_the_envelope_texts_with_the_response_model() {
    let guardrail = FakeGuardrail::echoing();
    let (result, events, stored) = process(
        text_stream(),
        &guardrail,
        Some(json!({"keep": 1})),
        None,
        false,
    )
    .await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(calls[0].inputs.texts, Some(strings(&["hello"])));
    assert_eq!(calls[0].inputs.model.as_deref(), Some("gpt-4o"));
    assert_eq!(calls[0].inputs.tool_calls, None);
    assert_eq!(
        stored,
        Some(json!({
            "keep": 1,
            "response": {"model": "gpt-4o", "output": [message_item()]}
        }))
    );
    assert_eq!(events, text_stream());
}

#[tokio::test]
async fn terminal_writes_the_returned_texts_into_the_envelope() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["HELLO"]}));
    let (result, events, _stored) = process(text_stream(), &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(
        events[2]["response"]["output"][0]["content"][0]["text"],
        json!("HELLO")
    );
    assert_eq!(events[0]["delta"], json!("he"));
    assert_eq!(events[1]["delta"], json!("llo"));
}

#[tokio::test]
async fn terminal_syncs_the_earlier_events_when_delivery_is_on() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["HELLO"]}));
    let (result, events, _stored) = process(text_stream(), &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(events[0]["delta"], json!("HELLO"));
    assert_eq!(events[1]["delta"], json!(""));
    assert_eq!(
        events[2]["response"]["output"][0]["content"][0]["text"],
        json!("HELLO")
    );
}

#[tokio::test]
async fn terminal_syncs_nothing_when_the_texts_come_back_unchanged() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["hello"]}));
    let (result, events, _stored) = process(text_stream(), &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(events, text_stream());
}

#[tokio::test]
async fn terminal_scans_the_envelope_tool_calls() {
    let guardrail = FakeGuardrail::echoing();
    let (result, _events, _stored) =
        process(tool_call_stream(), &guardrail, None, None, false).await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls[0].inputs.texts, Some(strings(&["hello"])));
    assert_eq!(calls[0].inputs.model, None);
    let tool_calls = serde_json::to_value(calls[0].inputs.tool_calls.clone().unwrap()).unwrap();
    assert_eq!(tool_calls[0]["type"], json!("function"));
    assert_eq!(tool_calls[0]["function"]["name"], json!("lookup"));
    assert_eq!(tool_calls[0]["function"]["arguments"], json!("{\"q\":1}"));
    assert_eq!(tool_calls[0]["index"], json!(1));
}

#[tokio::test]
async fn terminal_delivers_a_rewritten_tool_call() {
    let reply = json!({"tool_calls": [
        {"type": "function", "function": {"name": "lookup", "arguments": "{\"q\":2}"}, "index": 1}
    ]});
    let guardrail = FakeGuardrail::replying(reply);
    let (result, events, _stored) = process(tool_call_stream(), &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(
        events[5]["response"]["output"][1]["arguments"],
        json!("{\"q\":2}")
    );
    assert_eq!(events[0]["item"]["name"], json!("lookup"));
    assert_eq!(events[0]["item"]["arguments"], json!(""));
    assert_eq!(events[1]["delta"], json!("{\"q\":2}"));
    assert_eq!(events[2]["delta"], json!(""));
    assert_eq!(events[3]["arguments"], json!("{\"q\":2}"));
    assert_eq!(events[4]["item"]["arguments"], json!("{\"q\":2}"));
}

#[tokio::test]
async fn terminal_ignores_a_guardrail_that_returns_no_tool_calls() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["hello"]}));
    let (result, events, _stored) = process(tool_call_stream(), &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(
        events[5]["response"]["output"][1]["arguments"],
        json!("{\"q\":1}")
    );
    assert_eq!(events[1]["delta"], json!("{\"q\""));
    assert_eq!(events[2]["delta"], json!(":1}"));
    assert_eq!(events[3]["arguments"], json!("{\"q\":1}"));
}

#[tokio::test]
async fn terminal_reports_an_undeliverable_tool_call_rewrite() {
    let events = json!([
        delta("m1", 0, 0, "hi"),
        {"type": "response.completed", "response": {"output": [function_call_item("{}")]}}
    ]);
    let reply = json!({"tool_calls": [
        {"type": "function", "function": {"name": "lookup", "arguments": "{\"q\":2}"}, "index": 0}
    ]});
    let guardrail = FakeGuardrail::replying(reply);
    let (result, _events, _stored) = process(events, &guardrail, None, None, true).await;
    let error = result.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Guardrail 'guard' rewrote the streamed response but the rewrite cannot be written back to the stream: no stream event carries the rewritten call_id call_1"
    );
}

#[tokio::test]
async fn terminal_passes_a_guardrail_failure_through() {
    let failure = GuardrailFailure::Other("blocked".to_owned());
    let guardrail = FakeGuardrail::failing(failure.clone());
    let (result, events, _stored) = process(text_stream(), &guardrail, None, None, false).await;
    assert!(matches!(result, Err(StreamOutputError::Guardrail(source)) if source == failure));
    assert_eq!(events, text_stream());
}

#[tokio::test]
async fn terminal_falls_back_to_the_string_so_far_when_the_envelope_has_nothing_to_scan() {
    let events = json!([
        delta("m1", 0, 0, "he"),
        delta("m1", 0, 0, "llo"),
        {"type": "response.completed", "response": {"model": "gpt-4o", "output": []}}
    ]);
    let guardrail = FakeGuardrail::echoing();
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, false).await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].inputs.texts, Some(strings(&["hello"])));
    assert_eq!(calls[0].inputs.model.as_deref(), Some("gpt-4o"));
    assert!(calls[0].request_data.is_empty());
    assert_eq!(after, events);
}

#[tokio::test]
async fn terminal_spreads_the_fallback_text_when_delivery_is_on() {
    let events = json!([
        delta("m1", 0, 0, "he"),
        delta("m1", 0, 0, "llo"),
        {"type": "response.completed", "response": {"output": []}}
    ]);
    let guardrail = FakeGuardrail::replying(json!({"texts": ["HELLO"]}));
    let (result, after, _stored) = process(events, &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(after[0]["delta"], json!("HELLO"));
    assert_eq!(after[1]["delta"], json!(""));
}

#[tokio::test]
async fn terminal_leaves_the_fallback_text_alone_when_delivery_is_off() {
    let events = json!([
        delta("m1", 0, 0, "he"),
        delta("m1", 0, 0, "llo"),
        {"type": "response.completed", "response": {"output": []}}
    ]);
    let guardrail = FakeGuardrail::replying(json!({"texts": ["HELLO"]}));
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(guardrail.calls().len(), 1);
    assert_eq!(after, events);
}

#[rstest]
#[case::missing(json!([{"type": "response.completed"}]))]
#[case::not_an_object(json!([{"type": "response.failed", "response": "text"}]))]
#[tokio::test]
async fn terminal_treats_a_response_that_is_not_an_object_as_empty(#[case] events: Value) {
    let guardrail = FakeGuardrail::echoing();
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, true).await;
    result.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(after, events);
}

#[rstest]
#[case::failed("response.failed")]
#[case::incomplete("response.incomplete")]
#[tokio::test]
async fn terminal_scans_a_partial_envelope(#[case] kind: &str) {
    let events = json!([
        delta("m1", 0, 0, "he"),
        {"type": kind, "response": {"output": [message_item()]}}
    ]);
    let guardrail = FakeGuardrail::echoing();
    let (result, _after, _stored) = process(events, &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(guardrail.calls()[0].inputs.texts, Some(strings(&["hello"])));
}

#[tokio::test]
async fn terminal_starts_from_empty_request_data_when_none_is_given() {
    let guardrail = FakeGuardrail::echoing();
    let (result, _events, stored) = process(text_stream(), &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(stored, None);
    let sent = serde_json::to_value(&guardrail.calls()[0].request_data).unwrap();
    assert_eq!(
        sent,
        json!({"response": {"model": "gpt-4o", "output": [message_item()]}})
    );
}

#[tokio::test]
async fn terminal_adds_the_user_key_metadata_to_the_request_data() {
    let guardrail = FakeGuardrail::echoing();
    let (result, _events, stored) = process(
        text_stream(),
        &guardrail,
        Some(json!({})),
        Some(json!({"user_id": "u1"})),
        false,
    )
    .await;
    result.unwrap();
    assert_eq!(
        stored.unwrap()["litellm_metadata"],
        json!({"user_api_key_user_id": "u1"})
    );
}

#[tokio::test]
async fn terminal_keeps_a_response_entry_the_request_data_already_has() {
    let guardrail = FakeGuardrail::echoing();
    let (result, _events, stored) = process(
        text_stream(),
        &guardrail,
        Some(json!({"response": "mine"})),
        None,
        false,
    )
    .await;
    result.unwrap();
    assert_eq!(stored, Some(json!({"response": "mine"})));
}

#[rstest]
#[case::empty(json!({"model": "", "output": [{"type": "message", "content": [{"text": "hello"}]}]}))]
#[case::not_text(json!({"model": 5, "output": [{"type": "message", "content": [{"text": "hello"}]}]}))]
#[case::absent(json!({"output": [{"type": "message", "content": [{"text": "hello"}]}]}))]
#[tokio::test]
async fn terminal_leaves_the_model_out_unless_it_is_non_empty_text(#[case] response: Value) {
    let events = json!([
        delta("m1", 0, 0, "he"),
        {"type": "response.completed", "response": response}
    ]);
    let guardrail = FakeGuardrail::echoing();
    let (result, _after, _stored) = process(events, &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(guardrail.calls()[0].inputs.model, None);
}

#[tokio::test]
async fn terminal_fails_before_syncing_when_more_texts_come_back_than_were_scanned() {
    let guardrail = FakeGuardrail::replying(json!({"texts": ["A", "B"]}));
    let (result, events, _stored) = process(text_stream(), &guardrail, None, None, true).await;
    assert!(matches!(
        result,
        Err(StreamOutputError::Rewrite(
            OutputRewriteError::TooManyTexts { index: 1, .. }
        ))
    ));
    assert_eq!(
        events[2]["response"]["output"][0]["content"][0]["text"],
        json!("A")
    );
    assert_eq!(events[0]["delta"], json!("he"));
    assert_eq!(events[1]["delta"], json!("llo"));
}

#[tokio::test]
async fn item_done_scans_the_one_tool_call_without_texts_or_model() {
    let events = json!([
        {"type": "response.output_item.done", "output_index": 2, "item": function_call_item("{}")}
    ]);
    let guardrail = FakeGuardrail::echoing();
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, false).await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(calls[0].inputs.texts, None);
    assert_eq!(calls[0].inputs.model, None);
    assert!(calls[0].request_data.is_empty());
    let tool_calls = serde_json::to_value(calls[0].inputs.tool_calls.clone().unwrap()).unwrap();
    assert_eq!(tool_calls[0]["index"], json!(2));
    assert_eq!(tool_calls[0]["function"]["name"], json!("lookup"));
    assert_eq!(tool_calls[0]["function"]["arguments"], json!("{}"));
    assert_eq!(after, events);
}

#[tokio::test]
async fn item_done_reads_a_custom_tool_call_with_a_zero_index() {
    let events = json!([
        {"type": "response.output_item.done", "item": {"type": "custom_tool_call", "id": "ctc_1", "call_id": "call_2", "name": "grammar", "input": "raw"}}
    ]);
    let guardrail = FakeGuardrail::echoing();
    let (result, _after, _stored) = process(events, &guardrail, None, None, false).await;
    result.unwrap();
    let calls = guardrail.calls();
    let tool_calls = serde_json::to_value(calls[0].inputs.tool_calls.clone().unwrap()).unwrap();
    assert_eq!(tool_calls[0]["index"], json!(0));
    assert_eq!(tool_calls[0]["function"]["name"], json!("grammar"));
    assert_eq!(tool_calls[0]["function"]["arguments"], json!("raw"));
}

#[rstest]
#[case::text(json!("2"))]
#[case::negative(json!(-1))]
#[case::fraction(json!(1.5))]
#[tokio::test]
async fn item_done_uses_zero_when_the_output_index_is_not_an_integer(#[case] index: Value) {
    let events = json!([
        {"type": "response.output_item.done", "output_index": index, "item": function_call_item("{}")}
    ]);
    let guardrail = FakeGuardrail::echoing();
    let (result, _after, _stored) = process(events, &guardrail, None, None, false).await;
    result.unwrap();
    let calls = guardrail.calls();
    let tool_calls = serde_json::to_value(calls[0].inputs.tool_calls.clone().unwrap()).unwrap();
    assert_eq!(tool_calls[0]["index"], json!(0));
}

#[tokio::test]
async fn item_done_makes_no_call_for_an_item_that_is_not_a_tool_call() {
    let events = json!([
        {"type": "response.output_item.done", "output_index": 0, "item": message_item()}
    ]);
    let guardrail = FakeGuardrail::echoing();
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, false).await;
    result.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(after, events);
}

#[rstest]
#[case::no_item(json!([{"type": "response.output_item.done"}]))]
#[case::not_an_object(json!([{"type": "response.output_item.done", "item": "bare"}]))]
#[tokio::test]
async fn item_done_makes_no_call_without_a_usable_item(#[case] events: Value) {
    let guardrail = FakeGuardrail::echoing();
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, false).await;
    result.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(after, events);
}

#[tokio::test]
async fn item_done_falls_back_to_the_string_so_far_when_delivery_is_on() {
    let events = json!([
        delta("m1", 0, 0, "hi"),
        {"type": "response.output_item.done", "output_index": 0, "item": function_call_item("{}")}
    ]);
    let guardrail = FakeGuardrail::replying(json!({"texts": ["HI"]}));
    let (result, after, _stored) = process(events, &guardrail, None, None, true).await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].inputs.tool_calls.as_ref().unwrap().len(), 1);
    assert_eq!(calls[1].inputs.texts, Some(strings(&["hi"])));
    assert_eq!(after[0]["delta"], json!("HI"));
}

#[tokio::test]
async fn item_done_leaves_the_text_alone_when_delivery_is_off() {
    let events = json!([
        delta("m1", 0, 0, "hi"),
        {"type": "response.output_item.done", "output_index": 0, "item": function_call_item("{}")}
    ]);
    let guardrail = FakeGuardrail::replying(json!({"texts": ["HI"]}));
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(guardrail.calls().len(), 1);
    assert_eq!(after, events);
}

#[rstest]
#[case::empty_delta(json!([{"type": "response.output_text.delta", "output_index": 0, "content_index": 0, "delta": ""}]))]
#[case::no_text_event(json!([{"type": "response.created"}]))]
#[tokio::test]
async fn fallback_makes_no_call_for_an_empty_string_so_far(#[case] events: Value) {
    let guardrail = FakeGuardrail::echoing();
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, true).await;
    result.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(after, events);
}

#[tokio::test]
async fn fallback_scans_the_string_so_far_once_and_spreads_the_first_text() {
    let events = json!([delta("m1", 0, 0, "he"), delta("m1", 0, 1, "y")]);
    let guardrail = FakeGuardrail::replying(json!({"texts": ["HI", "there"]}));
    let (result, after, _stored) = process(events, &guardrail, None, None, true).await;
    result.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].inputs.texts, Some(strings(&["hey"])));
    assert_eq!(after[0]["delta"], json!("HI"));
    assert_eq!(after[1]["delta"], json!(""));
}

#[tokio::test]
async fn fallback_leaves_the_events_alone_when_delivery_is_off() {
    let events = json!([delta("m1", 0, 0, "he"), delta("m1", 0, 1, "y")]);
    let guardrail = FakeGuardrail::replying(json!({"texts": ["HI"]}));
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(guardrail.calls().len(), 1);
    assert_eq!(after, events);
}

#[tokio::test]
async fn fallback_leaves_a_single_unchanged_text_alone() {
    let events = json!([delta("m1", 0, 0, "he"), delta("m1", 0, 1, "y")]);
    let guardrail = FakeGuardrail::replying(json!({"texts": ["hey"]}));
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(after, events);
}

#[rstest]
#[case::no_texts(json!({}))]
#[case::empty_texts(json!({"texts": []}))]
#[tokio::test]
async fn fallback_leaves_an_empty_texts_answer_alone(#[case] reply: Value) {
    let events = json!([delta("m1", 0, 0, "he"), delta("m1", 0, 1, "y")]);
    let guardrail = FakeGuardrail::replying(reply);
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, true).await;
    result.unwrap();
    assert_eq!(after, events);
}

#[tokio::test]
async fn fallback_uses_the_model_of_the_last_event_response() {
    let events = json!([{
        "type": "response.output_text.delta",
        "output_index": 0,
        "content_index": 0,
        "delta": "hi",
        "response": {"model": "m"}
    }]);
    let guardrail = FakeGuardrail::echoing();
    let (result, _after, _stored) = process(events, &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(guardrail.calls()[0].inputs.model.as_deref(), Some("m"));
}

#[rstest]
#[case::empty(json!({"model": ""}))]
#[case::not_text(json!({"model": 5}))]
#[case::not_an_object(json!("text"))]
#[tokio::test]
async fn fallback_leaves_the_model_out_unless_it_is_non_empty_text(#[case] response: Value) {
    let events = json!([{
        "type": "response.output_text.delta",
        "output_index": 0,
        "content_index": 0,
        "delta": "hi",
        "response": response
    }]);
    let guardrail = FakeGuardrail::echoing();
    let (result, _after, _stored) = process(events, &guardrail, None, None, false).await;
    result.unwrap();
    assert_eq!(guardrail.calls()[0].inputs.model, None);
}

#[tokio::test]
async fn fallback_passes_a_guardrail_failure_through() {
    let failure = GuardrailFailure::Other("blocked".to_owned());
    let guardrail = FakeGuardrail::failing(failure.clone());
    let (result, _after, _stored) = process(
        json!([delta("m1", 0, 0, "hi")]),
        &guardrail,
        None,
        None,
        true,
    )
    .await;
    assert!(matches!(result, Err(StreamOutputError::Guardrail(source)) if source == failure));
}

#[tokio::test]
async fn fallback_returns_for_an_empty_stream() {
    let guardrail = FakeGuardrail::echoing();
    let (result, after, _stored) = process(json!([]), &guardrail, None, None, true).await;
    result.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(after, json!([]));
}

#[tokio::test]
async fn fallback_returns_when_the_last_event_is_not_an_object() {
    let events = json!([delta("m1", 0, 0, "hi"), "bare"]);
    let guardrail = FakeGuardrail::echoing();
    let (result, after, _stored) = process(events.clone(), &guardrail, None, None, true).await;
    result.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(after, events);
}
