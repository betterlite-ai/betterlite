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

use super::{VideoTranslationError, process_input_messages, process_output_response};

#[derive(Clone, Debug, PartialEq)]
struct Recorded {
    inputs: GuardrailApiInputs,
    input_type: GuardrailInputType,
    request_data: BTreeMap<String, GuardrailValue>,
}

struct FakeGuardrail {
    reply: Option<Vec<String>>,
    failure: Option<GuardrailFailure>,
    touch: bool,
    calls: Mutex<Vec<Recorded>>,
}

impl FakeGuardrail {
    fn replying(texts: &[&str]) -> Self {
        Self {
            reply: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
            failure: None,
            touch: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn touching(texts: &[&str]) -> Self {
        Self {
            reply: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
            failure: None,
            touch: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn without_texts() -> Self {
        Self {
            reply: None,
            failure: None,
            touch: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(failure: GuardrailFailure) -> Self {
        Self {
            reply: None,
            failure: Some(failure),
            touch: false,
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
        if self.touch {
            request_data.insert(
                "guardrail_touched".to_owned(),
                GuardrailValue::Boolean(true),
            );
        }
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

fn prompt_of(data: &BTreeMap<String, GuardrailValue>) -> &GuardrailValue {
    data.get("prompt").unwrap()
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
async fn input_text_prompt_sends_one_text_with_request_type_and_replaces_the_prompt() {
    let guardrail = FakeGuardrail::replying(&["rewritten", "dropped"]);
    let mut data = body(r#"{"model":"sora-2","prompt":"hello"}"#);
    let original = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(
        prompt_of(&data),
        &GuardrailValue::Text("rewritten".to_owned())
    );
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Request);
    assert_eq!(calls[0].inputs.texts, Some(vec!["hello".to_owned()]));
    assert_eq!(calls[0].inputs.model, Some("sora-2".to_owned()));
    assert_eq!(calls[0].request_data, original);
}

#[rstest]
#[case::text(r#"{"model":"sora-2","prompt":"hi"}"#, Some("sora-2"))]
#[case::empty(r#"{"model":"","prompt":"hi"}"#, Some(""))]
#[case::integer(r#"{"model":7,"prompt":"hi"}"#, None)]
#[case::null(r#"{"model":null,"prompt":"hi"}"#, None)]
#[case::absent(r#"{"prompt":"hi"}"#, None)]
#[tokio::test]
async fn input_model_is_sent_for_any_text_including_empty(
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
async fn input_empty_prompt_is_still_scanned() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut data = body(r#"{"prompt":""}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls()[0].inputs.texts, Some(vec![String::new()]));
    assert_eq!(
        prompt_of(&data),
        &GuardrailValue::Text("rewritten".to_owned())
    );
}

#[tokio::test]
async fn input_absent_reply_keeps_the_prompt() {
    let guardrail = FakeGuardrail::without_texts();
    let mut data = body(r#"{"prompt":"hello"}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(prompt_of(&data), &GuardrailValue::Text("hello".to_owned()));
}

#[tokio::test]
async fn input_empty_reply_keeps_the_prompt() {
    let guardrail = FakeGuardrail::replying(&[]);
    let mut data = body(r#"{"prompt":"hello"}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(prompt_of(&data), &GuardrailValue::Text("hello".to_owned()));
}

#[tokio::test]
async fn input_one_empty_text_replaces_the_prompt_with_an_empty_string() {
    let guardrail = FakeGuardrail::replying(&[""]);
    let mut data = body(r#"{"prompt":"hello"}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(prompt_of(&data), &GuardrailValue::Text(String::new()));
}

#[rstest]
#[case::missing(r#"{"model":"sora-2"}"#)]
#[case::null(r#"{"prompt":null}"#)]
#[case::integer(r#"{"prompt":7}"#)]
#[case::boolean(r#"{"prompt":true}"#)]
#[case::list(r#"{"prompt":["a"]}"#)]
#[case::object(r#"{"prompt":{"text":"a"}}"#)]
#[tokio::test]
async fn input_missing_null_or_non_text_prompt_makes_no_call(#[case] raw: &str) {
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = body(raw);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data, untouched);
}

#[tokio::test]
async fn input_writes_no_log_line() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut data = body(r#"{"model":"sora-2","prompt":"hello"}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    let mut without_prompt = body(r#"{"model":"sora-2"}"#);
    process_input_messages(&mut without_prompt, &guardrail, None)
        .await
        .unwrap();
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(written.is_empty(), "{written}");
}

#[tokio::test]
async fn input_request_data_change_made_by_the_guardrail_stays_on_the_body() {
    let guardrail = FakeGuardrail::touching(&["rewritten"]);
    let mut data = body(r#"{"prompt":"hello"}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(
        data.get("guardrail_touched"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(
        prompt_of(&data),
        &GuardrailValue::Text("rewritten".to_owned())
    );
    assert_eq!(
        guardrail.calls()[0].request_data.get("guardrail_touched"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[tokio::test]
async fn input_guardrail_failure_is_returned_and_leaves_the_body_alone() {
    let failure = GuardrailFailure::Other("blocked by fake".to_owned());
    let guardrail = FakeGuardrail::failing(failure.clone());
    let mut data = body(r#"{"prompt":"hello"}"#);
    let untouched = data.clone();
    let error = process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap_err();
    match error {
        VideoTranslationError::Guardrail(returned) => assert_eq!(returned, failure),
    }
    assert_eq!(data, untouched);
}

#[tokio::test]
async fn output_changes_nothing_and_writes_no_log() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let response = value(r#"{"id":"video-1","status":"queued"}"#);
    let untouched = response.clone();
    process_output_response(&response);
    assert_eq!(response, untouched);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(written.is_empty(), "{written}");
}
