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

use super::{TranscriptionTranslationError, process_input_messages, process_output_response};

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

fn text_of(response: &GuardrailValue) -> Option<&GuardrailValue> {
    let GuardrailValue::Object(fields) = response else {
        return None;
    };
    fields.get("text")
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
async fn input_processing_makes_no_call_and_logs_at_debug() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let data = body(r#"{"model":"whisper-1","file":"audio.mp3"}"#);
    let untouched = data.clone();
    process_input_messages(&data);
    assert_eq!(data, untouched);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains("Input processing not applicable (input is audio file, not text)"),
        "{written}"
    );
}

#[tokio::test]
async fn output_text_sends_one_text_with_response_type_and_rewrites_it() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["rewritten", "dropped"]);
    let mut response = value(r#"{"model":"whisper-1","text":"hello"}"#);
    let original = response.clone();
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        text_of(&response),
        Some(&GuardrailValue::Text("rewritten".to_owned()))
    );
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(calls[0].inputs.texts, Some(vec!["hello".to_owned()]));
    assert_eq!(calls[0].inputs.model, Some("whisper-1".to_owned()));
    assert_eq!(
        calls[0].request_data,
        BTreeMap::from([("response".to_owned(), original)])
    );
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written
            .contains("Applied guardrail to transcribed text. Original length: 5, New length: 9"),
        "{written}"
    );
}

#[rstest]
#[case::text(r#"{"model":"whisper-1","text":"hi"}"#, Some("whisper-1"))]
#[case::empty(r#"{"model":"","text":"hi"}"#, None)]
#[case::integer(r#"{"model":7,"text":"hi"}"#, None)]
#[case::null(r#"{"model":null,"text":"hi"}"#, None)]
#[case::absent(r#"{"text":"hi"}"#, None)]
#[tokio::test]
async fn output_model_is_sent_only_when_it_is_a_non_empty_text(
    #[case] raw: &str,
    #[case] expected: Option<&str>,
) {
    let guardrail = FakeGuardrail::replying(&["hi"]);
    let mut response = value(raw);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls()[0].inputs.model.as_deref(), expected);
}

#[tokio::test]
async fn output_absent_reply_keeps_the_text() {
    let guardrail = FakeGuardrail::without_texts();
    let mut response = value(r#"{"text":"hello"}"#);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        text_of(&response),
        Some(&GuardrailValue::Text("hello".to_owned()))
    );
}

#[tokio::test]
async fn output_empty_reply_keeps_the_text() {
    let guardrail = FakeGuardrail::replying(&[]);
    let mut response = value(r#"{"text":"hello"}"#);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        text_of(&response),
        Some(&GuardrailValue::Text("hello".to_owned()))
    );
}

#[tokio::test]
async fn output_one_empty_text_sets_the_text_to_empty() {
    let guardrail = FakeGuardrail::replying(&[""]);
    let mut response = value(r#"{"text":"hello"}"#);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        text_of(&response),
        Some(&GuardrailValue::Text(String::new()))
    );
}

#[tokio::test]
async fn output_empty_transcript_is_still_sent() {
    let guardrail = FakeGuardrail::replying(&["hello"]);
    let mut response = value(r#"{"text":""}"#);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls()[0].inputs.texts, Some(vec![String::new()]));
    assert_eq!(
        text_of(&response),
        Some(&GuardrailValue::Text("hello".to_owned()))
    );
}

#[rstest]
#[case::no_text(r#"{"model":"whisper-1"}"#)]
#[case::null_text(r#"{"text":null}"#)]
#[case::not_an_object(r#""transcript""#)]
#[tokio::test]
async fn output_without_text_makes_no_call_and_leaves_the_request_data(#[case] raw: &str) {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut response = value(raw);
    let untouched = response.clone();
    let mut request_data = BTreeMap::new();
    process_output_response(
        &mut response,
        &guardrail,
        None,
        None,
        Some(&mut request_data),
    )
    .await
    .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(response, untouched);
    assert!(request_data.is_empty());
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains("No text in response to process"),
        "{written}"
    );
}

// serde_json reads a float into an untagged Object under arbitrary_precision, so the float is built directly
#[rstest]
#[case::integer(value("7"), "int")]
#[case::boolean(value("true"), "bool")]
#[case::float(GuardrailValue::Decimal(1.5), "float")]
#[case::object(value(r#"{"value":"x"}"#), "dict")]
#[case::array(value(r#"["x"]"#), "list")]
#[tokio::test]
async fn output_non_text_text_makes_no_call_and_logs_at_debug(
    #[case] text: GuardrailValue,
    #[case] kind: &str,
) {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut response = GuardrailValue::Object(BTreeMap::from([("text".to_owned(), text)]));
    let untouched = response.clone();
    let mut request_data = BTreeMap::new();
    process_output_response(
        &mut response,
        &guardrail,
        None,
        None,
        Some(&mut request_data),
    )
    .await
    .unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(response, untouched);
    assert!(request_data.is_empty());
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains(&format!("Unexpected text type: {kind}. Expected string.")),
        "{written}"
    );
}

#[tokio::test]
async fn output_non_text_text_writes_no_warn_line() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let guardrail = FakeGuardrail::replying(&["unused"]);
    let mut response = GuardrailValue::Object(BTreeMap::from([("text".to_owned(), value("7"))]));
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(written.is_empty(), "{written}");
}

#[tokio::test]
async fn output_adds_response_and_user_metadata_to_the_given_request_data() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut response = value(r#"{"text":"hello"}"#);
    let original = response.clone();
    let user_api_key_dict = body(r#"{"user_id":"u-1","team_id":null,"_internal":"hidden"}"#);
    let mut request_data = body(r#"{"model":"whisper-1"}"#);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        Some(&user_api_key_dict),
        Some(&mut request_data),
    )
    .await
    .unwrap();
    let recorded = guardrail.calls().remove(0).request_data;
    assert_eq!(
        recorded.get("litellm_metadata"),
        Some(&value(r#"{"user_api_key_user_id":"u-1"}"#))
    );
    assert_eq!(
        recorded.get("model"),
        Some(&GuardrailValue::Text("whisper-1".to_owned()))
    );
    assert_eq!(recorded.get("response"), Some(&original));
    assert_eq!(
        request_data.get("litellm_metadata"),
        Some(&value(r#"{"user_api_key_user_id":"u-1"}"#))
    );
    assert_eq!(request_data.get("response"), Some(&original));
}

#[tokio::test]
async fn output_builds_the_request_data_when_none_is_given() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut response = value(r#"{"text":"hello"}"#);
    let original = response.clone();
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    let recorded = guardrail.calls().remove(0).request_data;
    assert_eq!(
        recorded,
        BTreeMap::from([("response".to_owned(), original)])
    );
}

#[tokio::test]
async fn output_builds_a_fresh_map_with_user_metadata_when_none_is_given() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut response = value(r#"{"text":"hello"}"#);
    let original = response.clone();
    let user_api_key_dict = body(r#"{"user_id":"u-1"}"#);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        Some(&user_api_key_dict),
        None,
    )
    .await
    .unwrap();
    let recorded = guardrail.calls().remove(0).request_data;
    assert_eq!(recorded.get("response"), Some(&original));
    assert_eq!(
        recorded.get("litellm_metadata"),
        Some(&value(r#"{"user_api_key_user_id":"u-1"}"#))
    );
}

#[tokio::test]
async fn output_keeps_an_existing_response_and_metadata_key() {
    let guardrail = FakeGuardrail::replying(&["rewritten"]);
    let mut response = value(r#"{"text":"hello"}"#);
    let user_api_key_dict = body(r#"{"user_id":"u-1"}"#);
    let mut request_data = body(r#"{"response":"kept","litellm_metadata":null}"#);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        Some(&user_api_key_dict),
        Some(&mut request_data),
    )
    .await
    .unwrap();
    let recorded = guardrail.calls().remove(0).request_data;
    assert_eq!(
        recorded.get("response"),
        Some(&GuardrailValue::Text("kept".to_owned()))
    );
    assert_eq!(
        recorded.get("litellm_metadata"),
        Some(&GuardrailValue::Null)
    );
}

#[tokio::test]
async fn output_guardrail_failure_is_returned_and_leaves_the_response_alone() {
    let failure = GuardrailFailure::Other("blocked by fake".to_owned());
    let guardrail = FakeGuardrail::failing(failure.clone());
    let mut response = value(r#"{"text":"hello"}"#);
    let untouched = response.clone();
    let error = process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap_err();
    match error {
        TranscriptionTranslationError::Guardrail(returned) => assert_eq!(returned, failure),
    }
    assert_eq!(response, untouched);
}
