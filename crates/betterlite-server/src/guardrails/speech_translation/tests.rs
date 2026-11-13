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

use super::{SpeechTranslationError, process_input_messages, process_output_response};

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
async fn input_text_input_sends_one_text_with_request_type_and_replaces_the_input() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["rewritten", "dropped"]);
    let mut data = body(r#"{"model":"tts-1","input":"hello"}"#);
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
    assert_eq!(calls[0].inputs.model, Some("tts-1".to_owned()));
    assert_eq!(calls[0].request_data, original);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains("Applied guardrail to input text. Original length: 5, New length: 9"),
        "{written}"
    );
}

#[tokio::test]
async fn input_applied_log_counts_characters_not_bytes() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["h\u{e9}llo"]);
    let mut data = body("{\"input\":\"a\u{e9}\"}");
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains("Original length: 2, New length: 5"),
        "{written}"
    );
}

#[rstest]
#[case::text(r#"{"model":"tts-1","input":"hi"}"#, Some("tts-1"))]
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
async fn input_absent_reply_keeps_the_input() {
    let guardrail = FakeGuardrail::without_texts();
    let mut data = body(r#"{"input":"hello"}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(input_of(&data), &GuardrailValue::Text("hello".to_owned()));
}

#[tokio::test]
async fn input_empty_reply_keeps_the_input() {
    let guardrail = FakeGuardrail::replying(&[]);
    let mut data = body(r#"{"input":"hello"}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(input_of(&data), &GuardrailValue::Text("hello".to_owned()));
}

#[tokio::test]
async fn input_one_empty_text_replaces_the_input_with_an_empty_string() {
    let guardrail = FakeGuardrail::replying(&[""]);
    let mut data = body(r#"{"input":"hello"}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(input_of(&data), &GuardrailValue::Text(String::new()));
}

#[tokio::test]
async fn input_empty_text_is_still_scanned() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut data = body(r#"{"input":""}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls()[0].inputs.texts, Some(vec![String::new()]));
    assert_eq!(
        input_of(&data),
        &GuardrailValue::Text("rewritten".to_owned())
    );
}

#[rstest]
#[case::missing(r#"{"model":"tts-1"}"#)]
#[case::null(r#"{"input":null}"#)]
#[tokio::test]
async fn input_missing_or_null_input_makes_no_call_and_logs_at_debug(#[case] raw: &str) {
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
        written.contains("No input text found in request data"),
        "{written}"
    );
}

// serde_json reads a float into an untagged Object under arbitrary_precision, so the float is built directly
#[rstest]
#[case::integer(value("7"), "int")]
#[case::boolean(value("true"), "bool")]
#[case::float(GuardrailValue::Decimal(1.5), "float")]
#[case::object(value(r#"{"text":"x"}"#), "dict")]
#[case::array(value(r#"["x"]"#), "list")]
#[tokio::test]
async fn input_non_text_input_makes_no_call_and_logs_at_debug(
    #[case] input: GuardrailValue,
    #[case] kind: &str,
) {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = BTreeMap::from([("input".to_owned(), input)]);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data, untouched);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains(&format!("Unexpected input type: {kind}. Expected string.")),
        "{written}"
    );
}

#[tokio::test]
async fn input_non_text_input_writes_no_warn_line() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut data = BTreeMap::from([("input".to_owned(), value("7"))]);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(written.is_empty(), "{written}");
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
        SpeechTranslationError::Guardrail(returned) => assert_eq!(returned, failure),
    }
    assert_eq!(data, untouched);
}

#[tokio::test]
async fn output_logs_the_skip_and_leaves_the_response_unchanged() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let response = value(r#"{"audio":"cmlwMyBhdWRpbw=="}"#);
    let untouched = response.clone();
    process_output_response(&response);
    assert_eq!(response, untouched);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains("Output processing not applicable (output is audio data, not text)"),
        "{written}"
    );
}
