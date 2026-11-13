#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use rstest::rstest;

use super::{OutputTranslationError, process_output_response};
use crate::guardrails::bedrock_passthrough::routing::Routed;

#[derive(Default)]
struct Recorded {
    calls: usize,
    texts: Vec<String>,
    model: Option<String>,
    input_type: Option<GuardrailInputType>,
    request_data: Option<BTreeMap<String, GuardrailValue>>,
}

enum Answer {
    Unchanged,
    Texts(Vec<String>),
    Fail,
}

struct Scripted {
    answer: Answer,
    recorded: Mutex<Recorded>,
}

impl Scripted {
    fn new(answer: Answer) -> Self {
        Self {
            answer,
            recorded: Mutex::new(Recorded::default()),
        }
    }

    fn calls(&self) -> usize {
        self.recorded.lock().expect("not poisoned").calls
    }

    fn texts(&self) -> Vec<String> {
        self.recorded.lock().expect("not poisoned").texts.clone()
    }

    fn model(&self) -> Option<String> {
        self.recorded.lock().expect("not poisoned").model.clone()
    }

    fn input_type(&self) -> Option<GuardrailInputType> {
        self.recorded.lock().expect("not poisoned").input_type
    }

    fn request_data(&self) -> BTreeMap<String, GuardrailValue> {
        self.recorded
            .lock()
            .expect("not poisoned")
            .request_data
            .clone()
            .expect("the guardrail ran")
    }

    fn record(
        &self,
        inputs: &GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
    ) {
        request_data.insert("guardrail_wrote".to_owned(), GuardrailValue::Boolean(true));
        let mut recorded = self.recorded.lock().expect("not poisoned");
        recorded.calls = recorded.calls.saturating_add(1);
        recorded.texts = inputs.texts.clone().unwrap_or_default();
        recorded.model = inputs.model.clone();
        recorded.input_type = Some(input_type);
        recorded.request_data = Some(request_data.clone());
    }
}

impl GuardrailContract for Scripted {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.record(&inputs, request_data, input_type);
        let result = match &self.answer {
            Answer::Unchanged => Ok(inputs),
            Answer::Texts(texts) => Ok(GuardrailApiInputs {
                texts: Some(texts.clone()),
                ..inputs
            }),
            Answer::Fail => Err(GuardrailFailure::Other("scripted failure".to_owned())),
        };
        ready(result)
    }
}

fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).expect("valid JSON fixture")
}

fn data(json: &str) -> BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(fields) = value(json) else {
        panic!("object fixture");
    };
    fields
}

fn response_texts(response: &GuardrailValue) -> Vec<String> {
    let GuardrailValue::Object(fields) = response else {
        return Vec::new();
    };
    let Some(GuardrailValue::Object(output)) = fields.get("output") else {
        return Vec::new();
    };
    let Some(GuardrailValue::Object(message)) = output.get("message") else {
        return Vec::new();
    };
    let Some(GuardrailValue::Array(content)) = message.get("content") else {
        return Vec::new();
    };
    let mut texts = Vec::new();
    for entry in content {
        if let GuardrailValue::Object(block) = entry
            && let Some(GuardrailValue::Text(text)) = block.get("text")
        {
            texts.push(text.clone());
        }
    }
    texts
}

const CONVERSE: &str = "model/anthropic.claude-v2/converse";

const RESPONSE: &str = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "one"}, {"toolUse": {"input": {"q": "tool"}}}]}}, "stopReason": "end_turn"}"#;

#[rstest]
#[case::invoke("model/x/invoke")]
#[case::invoke_stream("model/x/invoke-with-response-stream")]
#[tokio::test]
async fn a_non_converse_endpoint_routes_to_the_generic_handler(#[case] endpoint: &str) {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response = value(RESPONSE);
    let before = response.clone();
    let request_data = data(&format!(r#"{{"endpoint": "{endpoint}"}}"#));
    let routed = process_output_response(
        &mut response,
        &guardrail,
        None,
        BTreeMap::new(),
        Some(&request_data),
    )
    .await
    .expect("no guardrail call");
    assert_eq!(routed, Routed::Generic);
    assert_eq!(guardrail.calls(), 0);
    assert_eq!(response, before);
}

#[rstest]
#[case::converse(CONVERSE)]
#[case::empty("")]
#[tokio::test]
async fn a_converse_or_empty_endpoint_keeps_the_response(#[case] endpoint: &str) {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response = value(RESPONSE);
    let request_data = data(&format!(r#"{{"endpoint": "{endpoint}"}}"#));
    let routed = process_output_response(
        &mut response,
        &guardrail,
        None,
        BTreeMap::new(),
        Some(&request_data),
    )
    .await
    .expect("guardrail answers");
    assert_eq!(routed, Routed::Handled);
    assert_eq!(guardrail.calls(), 1);
}

#[tokio::test]
async fn no_request_data_counts_as_converse() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response = value(RESPONSE);
    let routed = process_output_response(&mut response, &guardrail, None, BTreeMap::new(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(routed, Routed::Handled);
    assert_eq!(guardrail.calls(), 1);
    assert_eq!(guardrail.model(), None);
    assert!(!guardrail.request_data().contains_key("litellm_metadata"));
}

#[rstest]
#[case::not_an_object("[]")]
#[case::no_output(r#"{"stopReason": "end_turn"}"#)]
#[case::output_not_an_object(r#"{"output": []}"#)]
#[case::message_not_an_object(r#"{"output": {"message": "hello"}}"#)]
#[case::content_not_a_list(r#"{"output": {"message": {"content": "hello"}}}"#)]
#[tokio::test]
async fn an_unexpected_response_shape_is_handled_without_a_call(#[case] fixture: &str) {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response = value(fixture);
    let routed = process_output_response(&mut response, &guardrail, None, BTreeMap::new(), None)
        .await
        .expect("no guardrail call");
    assert_eq!(routed, Routed::Handled);
    assert_eq!(guardrail.calls(), 0);
}

#[tokio::test]
async fn no_text_means_no_guardrail_call() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response =
        value(r#"{"output": {"message": {"content": [{"toolUse": {"input": "plain"}}]}}}"#);
    process_output_response(&mut response, &guardrail, None, BTreeMap::new(), None)
        .await
        .expect("no guardrail call");
    assert_eq!(guardrail.calls(), 0);
}

#[tokio::test]
async fn the_guardrail_receives_the_output_texts_and_the_response_type() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response = value(RESPONSE);
    process_output_response(&mut response, &guardrail, None, BTreeMap::new(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(guardrail.texts(), vec!["one", "tool"]);
    assert_eq!(guardrail.input_type(), Some(GuardrailInputType::Response));
}

#[tokio::test]
async fn returned_texts_are_written_back_into_the_content_blocks() {
    let guardrail = Scripted::new(Answer::Texts(vec!["ONE".to_owned(), "TOOL".to_owned()]));
    let mut response = value(RESPONSE);
    process_output_response(&mut response, &guardrail, None, BTreeMap::new(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(response_texts(&response), vec!["ONE"]);
    assert_eq!(
        serde_json::to_string(&response).expect("serializes"),
        r#"{"output":{"message":{"content":[{"text":"ONE"},{"toolUse":{"input":{"q":"TOOL"}}}],"role":"assistant"}},"stopReason":"end_turn"}"#
    );
}

#[tokio::test]
async fn user_metadata_becomes_litellm_metadata_on_a_copy() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response = value(RESPONSE);
    let request_data = data(r#"{"endpoint": "model/x/converse", "model": "anthropic.claude-v2"}"#);
    let user_metadata = data(r#"{"user_api_key_user_id": "u1"}"#);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        user_metadata,
        Some(&request_data),
    )
    .await
    .expect("guardrail answers");
    let seen = guardrail.request_data();
    assert_eq!(
        seen.get("litellm_metadata"),
        Some(&GuardrailValue::Object(data(
            r#"{"user_api_key_user_id": "u1"}"#
        )))
    );
    assert_eq!(
        seen.get("model").and_then(text),
        Some("anthropic.claude-v2")
    );
    assert_eq!(guardrail.model().as_deref(), Some("anthropic.claude-v2"));
    assert!(!request_data.contains_key("litellm_metadata"));
    assert!(!request_data.contains_key("guardrail_wrote"));
}

#[tokio::test]
async fn an_existing_litellm_metadata_is_kept() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response = value(RESPONSE);
    let request_data = data(
        r#"{"endpoint": "model/x/converse", "litellm_metadata": {"user_api_key_user_id": "kept"}}"#,
    );
    let user_metadata = data(r#"{"user_api_key_user_id": "dropped"}"#);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        user_metadata,
        Some(&request_data),
    )
    .await
    .expect("guardrail answers");
    let seen = guardrail.request_data();
    assert_eq!(
        seen.get("litellm_metadata"),
        Some(&GuardrailValue::Object(data(
            r#"{"user_api_key_user_id": "kept"}"#
        )))
    );
}

#[tokio::test]
async fn empty_user_metadata_adds_no_metadata() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response = value(RESPONSE);
    let request_data = data(r#"{"endpoint": "model/x/converse"}"#);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        BTreeMap::new(),
        Some(&request_data),
    )
    .await
    .expect("guardrail answers");
    assert!(!guardrail.request_data().contains_key("litellm_metadata"));
}

#[tokio::test]
async fn the_response_model_never_reaches_the_guardrail() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response = value(
        r#"{"model": "from-response", "output": {"message": {"content": [{"text": "one"}]}}}"#,
    );
    process_output_response(&mut response, &guardrail, None, BTreeMap::new(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(guardrail.model(), None);
}

#[tokio::test]
async fn a_guardrail_failure_is_returned() {
    let guardrail = Scripted::new(Answer::Fail);
    let mut response = value(RESPONSE);
    let error = process_output_response(&mut response, &guardrail, None, BTreeMap::new(), None)
        .await
        .expect_err("the failure comes back");
    assert!(matches!(error, OutputTranslationError::Guardrail(_)));
}

#[tokio::test]
async fn an_empty_reply_leaves_the_response_alone() {
    let guardrail = Scripted::new(Answer::Texts(Vec::new()));
    let before = value(RESPONSE);
    let mut response = before.clone();
    process_output_response(&mut response, &guardrail, None, BTreeMap::new(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(guardrail.calls(), 1);
    assert_eq!(response, before);
}

#[tokio::test]
async fn an_empty_request_model_is_not_sent() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut response = value(RESPONSE);
    let request_data = data(r#"{"endpoint": "model/x/converse", "model": ""}"#);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        BTreeMap::new(),
        Some(&request_data),
    )
    .await
    .expect("guardrail answers");
    assert_eq!(guardrail.calls(), 1);
    assert_eq!(guardrail.model(), None);
}

fn text(value: &GuardrailValue) -> Option<&str> {
    let GuardrailValue::Text(text) = value else {
        return None;
    };
    Some(text)
}
