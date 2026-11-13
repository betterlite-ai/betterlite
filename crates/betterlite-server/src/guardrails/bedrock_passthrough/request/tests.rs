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

use super::{RequestTranslationError, process_input_messages};
use crate::guardrails::bedrock_passthrough::routing::Routed;
use crate::guardrails::bedrock_passthrough::text_paths::{SkipFlags, collect_request_texts};

#[derive(Default)]
struct Recorded {
    calls: usize,
    texts: Vec<String>,
    model: Option<String>,
    input_type: Option<GuardrailInputType>,
    saw_endpoint: bool,
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

    fn saw_endpoint(&self) -> bool {
        self.recorded.lock().expect("not poisoned").saw_endpoint
    }

    fn record(
        &self,
        inputs: &GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
    ) {
        let saw_endpoint = request_data.contains_key("endpoint");
        request_data.insert("guardrail_saw".to_owned(), GuardrailValue::Boolean(true));
        let mut recorded = self.recorded.lock().expect("not poisoned");
        recorded.calls = recorded.calls.saturating_add(1);
        recorded.texts = inputs.texts.clone().unwrap_or_default();
        recorded.model = inputs.model.clone();
        recorded.input_type = Some(input_type);
        recorded.saw_endpoint = saw_endpoint;
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

fn data(json: &str) -> BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(fields) = serde_json::from_str(json).expect("valid JSON fixture")
    else {
        panic!("object fixture");
    };
    fields
}

fn collect(data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
    data.get("data").map_or_else(Vec::new, |body| {
        collect_request_texts(body, SkipFlags::default())
            .into_iter()
            .map(|(_, text)| text)
            .collect()
    })
}

#[rstest]
#[case::invoke("model/x/invoke")]
#[case::invoke_stream("model/x/invoke-with-response-stream")]
#[case::empty("")]
#[case::unrelated("v1/chat/completions")]
#[tokio::test]
async fn a_non_converse_endpoint_routes_to_the_generic_handler(#[case] endpoint: &str) {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut request = data(&format!(
        r#"{{"endpoint": "{endpoint}", "data": {{"messages": [{{"content": [{{"text": "hi"}}]}}]}}}}"#
    ));
    let before = request.clone();
    let routed = process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("no guardrail call");
    assert_eq!(routed, Routed::Generic);
    assert_eq!(guardrail.calls(), 0);
    assert_eq!(request, before);
}

#[tokio::test]
async fn a_converse_endpoint_without_a_body_is_handled_without_a_call() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut request = data(r#"{"endpoint": "model/x/converse"}"#);
    let routed = process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("no guardrail call");
    assert_eq!(routed, Routed::Handled);
    assert_eq!(guardrail.calls(), 0);
}

#[rstest]
#[case::messages_not_a_list(
    r#"{"endpoint": "model/x/converse", "data": {"messages": {"role": "user"}}}"#
)]
#[case::body_a_string(r#"{"endpoint": "model/x/converse", "data": "text"}"#)]
#[case::no_messages(
    r#"{"endpoint": "model/x/converse", "data": {"system": [{"text": "be nice"}]}}"#
)]
#[tokio::test]
async fn a_body_without_a_messages_array_is_handled_without_a_call(#[case] fixture: &str) {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut request = data(fixture);
    let routed = process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("no guardrail call");
    assert_eq!(routed, Routed::Handled);
    assert_eq!(guardrail.calls(), 0);
}

#[tokio::test]
async fn no_text_means_no_guardrail_call() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut request = data(
        r#"{"endpoint": "model/x/converse", "data": {"messages": [{"content": [{"text": ""}]}]}}"#,
    );
    let routed = process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("no guardrail call");
    assert_eq!(routed, Routed::Handled);
    assert_eq!(guardrail.calls(), 0);
}

#[tokio::test]
async fn the_guardrail_receives_the_texts_the_whole_data_and_the_request_type() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut request = data(
        r#"{"endpoint": "model/x/converse", "model": "anthropic.claude-v2", "data": {"system": [{"text": "be nice"}], "messages": [{"content": [{"text": "hello"}]}]}}"#,
    );
    let routed = process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(routed, Routed::Handled);
    assert_eq!(guardrail.texts(), vec!["be nice", "hello"]);
    assert_eq!(guardrail.model().as_deref(), Some("anthropic.claude-v2"));
    assert_eq!(guardrail.input_type(), Some(GuardrailInputType::Request));
    assert!(guardrail.saw_endpoint());
    assert_eq!(
        request.get("guardrail_saw"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(GuardrailValue::Text(String::new())))]
#[case::not_text(Some(GuardrailValue::SignedInteger(7)))]
#[tokio::test]
async fn the_guardrail_gets_no_model_unless_it_is_non_empty_text(
    #[case] model: Option<GuardrailValue>,
) {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut request = data(
        r#"{"endpoint": "model/x/converse", "data": {"messages": [{"content": [{"text": "hello"}]}]}}"#,
    );
    if let Some(value) = model {
        request.insert("model".to_owned(), value);
    }
    process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(guardrail.calls(), 1);
    assert_eq!(guardrail.model(), None);
}

#[tokio::test]
async fn returned_texts_are_written_back_by_position() {
    let guardrail = Scripted::new(Answer::Texts(vec!["ONE".to_owned(), "TWO".to_owned()]));
    let mut request = data(
        r#"{"endpoint": "model/x/converse", "data": {"messages": [{"content": [{"text": "one"}, {"text": "two"}]}]}}"#,
    );
    process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(collect(&request), vec!["ONE", "TWO"]);
}

#[tokio::test]
async fn a_shorter_reply_writes_the_first_texts_and_keeps_the_rest() {
    let guardrail = Scripted::new(Answer::Texts(vec!["ONE".to_owned()]));
    let mut request = data(
        r#"{"endpoint": "model/x/converse", "data": {"messages": [{"content": [{"text": "one"}, {"text": "two"}]}]}}"#,
    );
    process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(collect(&request), vec!["ONE", "two"]);
}

#[tokio::test]
async fn a_longer_reply_drops_the_extra_texts() {
    let guardrail = Scripted::new(Answer::Texts(vec![
        "ONE".to_owned(),
        "TWO".to_owned(),
        "THREE".to_owned(),
    ]));
    let mut request = data(
        r#"{"endpoint": "model/x/converse", "data": {"messages": [{"content": [{"text": "one"}, {"text": "two"}]}]}}"#,
    );
    process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(collect(&request), vec!["ONE", "TWO"]);
}

#[tokio::test]
async fn an_empty_reply_leaves_the_body_alone() {
    let guardrail = Scripted::new(Answer::Texts(Vec::new()));
    let mut request = data(
        r#"{"endpoint": "model/x/converse", "data": {"messages": [{"content": [{"text": "one"}]}]}}"#,
    );
    process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(collect(&request), vec!["one"]);
}

#[tokio::test]
async fn the_skip_flags_come_from_the_caller() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut request = data(
        r#"{"endpoint": "model/x/converse", "data": {"system": [{"text": "be nice"}], "messages": [{"content": [{"text": "hello", "toolUse": {"input": {"q": "query"}}}]}]}}"#,
    );
    process_input_messages(
        &mut request,
        &guardrail,
        SkipFlags {
            system: true,
            tool: true,
        },
        None,
    )
    .await
    .expect("guardrail answers");
    assert!(guardrail.texts().is_empty());
    assert_eq!(guardrail.calls(), 0);
}

#[tokio::test]
async fn a_guardrail_failure_is_returned() {
    let guardrail = Scripted::new(Answer::Fail);
    let mut request = data(
        r#"{"endpoint": "model/x/converse", "data": {"messages": [{"content": [{"text": "one"}]}]}}"#,
    );
    let error = process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect_err("the failure comes back");
    assert!(matches!(error, RequestTranslationError::Guardrail(_)));
}

#[tokio::test]
async fn a_trailing_slash_endpoint_is_still_converse() {
    let guardrail = Scripted::new(Answer::Unchanged);
    let mut request = data(
        r#"{"endpoint": "model/x/converse/", "data": {"messages": [{"content": [{"text": "one"}]}]}}"#,
    );
    let routed = process_input_messages(&mut request, &guardrail, SkipFlags::default(), None)
        .await
        .expect("guardrail answers");
    assert_eq!(routed, Routed::Handled);
    assert_eq!(guardrail.calls(), 1);
}
