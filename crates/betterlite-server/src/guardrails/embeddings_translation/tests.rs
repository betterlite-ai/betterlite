#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use tracing::subscriber::{DefaultGuard, set_default};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

use super::{EmbeddingsTranslationError, process_input_messages, process_output_response};

#[derive(Clone, Debug, PartialEq)]
struct Recorded {
    inputs: GuardrailApiInputs,
    input_type: GuardrailInputType,
    request_data: BTreeMap<String, GuardrailValue>,
}

struct FakeGuardrail {
    reply: Option<Vec<String>>,
    failure: Option<GuardrailFailure>,
    calls: Mutex<Vec<Recorded>>,
}

impl FakeGuardrail {
    fn replying(texts: &[&str]) -> Self {
        Self {
            reply: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn without_texts() -> Self {
        Self {
            reply: None,
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
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.calls.lock().unwrap().push(Recorded {
            inputs,
            input_type,
            request_data: request_data.clone(),
        });
        let reply = self.reply.clone();
        let failure = self.failure.clone();
        async move {
            if let Some(blocked) = failure {
                return Err(blocked);
            }
            Ok(GuardrailApiInputs {
                texts: reply,
                images: None,
                tools: None,
                tool_calls: None,
                structured_messages: None,
                model: None,
                stream_holdback_chars: None,
            })
        }
    }
}

fn body(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

fn input_of(data: &BTreeMap<String, GuardrailValue>) -> &GuardrailValue {
    data.get("input").unwrap()
}

fn capture_logs(level: tracing::Level) -> (tempfile::NamedTempFile, DefaultGuard) {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    (log, set_default(subscriber))
}

#[tokio::test]
async fn input_text_sends_one_text_and_replaces_the_input() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut data = body(r#"{"model":"text-embedding-3-small","input":"hello"}"#);
    let original = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(
        input_of(&data),
        &GuardrailValue::Text("rewritten".to_owned())
    );
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Request);
    assert_eq!(calls[0].inputs.texts, Some(vec!["hello".to_owned()]));
    assert_eq!(
        calls[0].inputs.model,
        Some("text-embedding-3-small".to_owned())
    );
    assert_eq!(calls[0].request_data, original);
}

#[rstest]
#[case::text(r#"{"model":"m","input":"hi"}"#, Some("m"))]
#[case::empty(r#"{"model":"","input":"hi"}"#, None)]
#[case::integer(r#"{"model":7,"input":"hi"}"#, None)]
#[case::null(r#"{"model":null,"input":"hi"}"#, None)]
#[case::absent(r#"{"input":"hi"}"#, None)]
#[tokio::test]
async fn input_model_is_sent_only_when_it_is_a_non_empty_text(
    #[case] raw: &str,
    #[case] expected: Option<&str>,
) {
    let guardrail = FakeGuardrail::replying(&["hi"]);
    let mut data = body(raw);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls()[0].inputs.model.as_deref(), expected);
}

#[tokio::test]
async fn input_text_absent_reply_keeps_the_input() {
    let guardrail = FakeGuardrail::without_texts();
    let mut data = body(r#"{"input":"hello"}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(input_of(&data), &GuardrailValue::Text("hello".to_owned()));
}

#[tokio::test]
async fn input_text_empty_reply_keeps_the_input() {
    let guardrail = FakeGuardrail::replying(&[]);
    let mut data = body(r#"{"input":"hello"}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(input_of(&data), &GuardrailValue::Text("hello".to_owned()));
}

#[tokio::test]
async fn input_array_of_text_sends_the_whole_array_and_replaces_it_whole() {
    let guardrail = FakeGuardrail::replying(&["x", "y", "z"]);
    let mut data = body(r#"{"model":"m","input":["a","b"]}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(input_of(&data), &value(r#"["x","y","z"]"#));
    let calls = guardrail.calls();
    assert_eq!(
        calls[0].inputs.texts,
        Some(vec!["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(calls[0].inputs.model, Some("m".to_owned()));
}

#[tokio::test]
async fn input_array_empty_reply_keeps_the_input() {
    let guardrail = FakeGuardrail::replying(&[]);
    let mut data = body(r#"{"input":["a","b"]}"#);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls().len(), 1);
    assert_eq!(data, untouched);
}

#[tokio::test]
async fn input_array_absent_reply_keeps_the_input() {
    let guardrail = FakeGuardrail::without_texts();
    let mut data = body(r#"{"input":["a","b"]}"#);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(data, untouched);
}

#[tokio::test]
async fn input_empty_array_makes_no_call() {
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = body(r#"{"input":[]}"#);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data, untouched);
}

#[rstest]
#[case::integers(r#"{"input":[1,2,3]}"#)]
#[case::number_arrays(r#"{"input":[[1],[2]]}"#)]
#[tokio::test]
async fn input_token_ids_make_no_call(#[case] raw: &str) {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = body(raw);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data, untouched);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains("Input is token IDs, skipping guardrail processing"),
        "{written}"
    );
}

#[rstest]
#[case::boolean(r#"{"input":[true]}"#, "bool")]
#[case::float(r#"{"input":[1.5]}"#, "float")]
#[case::object(r#"{"input":[{"x":1}]}"#, "dict")]
#[case::null(r#"{"input":[null]}"#, "NoneType")]
#[tokio::test]
async fn input_unexpected_list_item_type_warns_and_makes_no_call(
    #[case] raw: &str,
    #[case] kind: &str,
) {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = body(raw);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data, untouched);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains(&format!("Unexpected input list item type: {kind}")),
        "{written}"
    );
}

#[rstest]
#[case::later_integer(r#"{"input":["a",5]}"#, 1, "int")]
#[case::later_object(r#"{"input":["a",{"x":1}]}"#, 1, "dict")]
#[tokio::test]
async fn input_mixed_array_starting_with_text_returns_an_error(
    #[case] raw: &str,
    #[case] expected_index: usize,
    #[case] expected_kind: &str,
) {
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = body(raw);
    let untouched = data.clone();
    let error = process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap_err();
    match error {
        EmbeddingsTranslationError::NonTextItem { index, kind } => {
            assert_eq!(index, expected_index);
            assert_eq!(kind, expected_kind);
        }
        other @ EmbeddingsTranslationError::Guardrail(_) => {
            panic!("expected a non-text item error, got {other:?}")
        }
    }
    assert!(guardrail.calls().is_empty());
    assert_eq!(data, untouched);
}

#[rstest]
#[case::missing(r#"{"model":"m"}"#)]
#[case::null(r#"{"input":null}"#)]
#[tokio::test]
async fn input_missing_or_null_makes_no_call(#[case] raw: &str) {
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = body(raw);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data, untouched);
}

#[rstest]
#[case::integer(r#"{"input":7}"#, "int")]
#[case::boolean(r#"{"input":true}"#, "bool")]
#[case::float(r#"{"input":1.5}"#, "float")]
#[case::object(r#"{"input":{"a":1}}"#, "dict")]
#[tokio::test]
async fn input_unexpected_type_warns_and_makes_no_call(#[case] raw: &str, #[case] kind: &str) {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = body(raw);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data, untouched);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains(&format!("Unexpected input type: {kind}")),
        "{written}"
    );
}

#[tokio::test]
async fn input_guardrail_failure_is_returned_and_leaves_the_body_alone() {
    let failure = GuardrailFailure::Other("blocked by fake".to_owned());
    let guardrail = FakeGuardrail::failing(failure.clone());
    let mut data = body(r#"{"input":"hello"}"#);
    let untouched = data.clone();
    let error = process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap_err();
    match error {
        EmbeddingsTranslationError::Guardrail(returned) => assert_eq!(returned, failure),
        other @ EmbeddingsTranslationError::NonTextItem { .. } => {
            panic!("expected a guardrail failure, got {other:?}")
        }
    }
    assert_eq!(data, untouched);
}

#[tokio::test]
async fn output_logs_the_skip_and_changes_nothing() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let response = value(r#"{"data":[{"embedding":[0.1,0.2],"index":0}],"model":"m"}"#);
    process_output_response(&response);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains(
            "OpenAI Embeddings: Output response processing skipped - embeddings contain vectors, not text"
        ),
        "{written}"
    );
}
