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

use super::{CompletionTranslationError, process_input_messages, process_output_response};

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
async fn input_text_prompt_sends_one_text_and_replaces_the_prompt() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["rewritten", "dropped"]);
    let mut data = body(r#"{"model":"gpt-3.5-turbo-instruct","prompt":"hello"}"#);
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
    assert_eq!(
        calls[0].inputs.model,
        Some("gpt-3.5-turbo-instruct".to_owned())
    );
    assert_eq!(calls[0].request_data, original);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains("Applied guardrail to string prompt. Original length: 5, New length: 9"),
        "{written}"
    );
}

#[tokio::test]
async fn input_string_prompt_debug_counts_characters() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["h\u{e9}llo"]);
    let mut data = body("{\"prompt\":\"a\u{e9}\"}");
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
#[case::text(r#"{"model":"m","prompt":"hi"}"#, Some("m"))]
#[case::empty(r#"{"model":"","prompt":"hi"}"#, None)]
#[case::integer(r#"{"model":7,"prompt":"hi"}"#, None)]
#[case::null(r#"{"model":null,"prompt":"hi"}"#, None)]
#[case::absent(r#"{"prompt":"hi"}"#, None)]
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

#[tokio::test]
async fn input_array_prompt_sends_text_elements_and_rewrites_their_positions() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["A", "B"]);
    let mut data = body(r#"{"model":"m","prompt":["a",5,["x"],"b"]}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(prompt_of(&data), &value(r#"["A",5,["x"],"B"]"#));
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].inputs.texts,
        Some(vec!["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(calls[0].inputs.model, Some("m".to_owned()));
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains("Applied guardrail to prompt[0]. Original length: 1, New length: 1"),
        "{written}"
    );
    assert!(
        written.contains("Applied guardrail to prompt[3]. Original length: 1, New length: 1"),
        "{written}"
    );
}

#[rstest]
#[case::empty_array(r#"{"prompt":[]}"#)]
#[case::integers(r#"{"prompt":[1,2]}"#)]
#[case::number_arrays(r#"{"prompt":[[1],[2]]}"#)]
#[tokio::test]
async fn input_array_prompt_without_text_elements_makes_no_call(#[case] raw: &str) {
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
async fn input_short_array_reply_leaves_the_later_prompts_alone() {
    let guardrail = FakeGuardrail::replying(&["A"]);
    let mut data = body(r#"{"prompt":["a","b","c"]}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(prompt_of(&data), &value(r#"["A","b","c"]"#));
}

#[tokio::test]
async fn input_long_array_reply_drops_its_extra_texts() {
    let guardrail = FakeGuardrail::replying(&["A", "B"]);
    let mut data = body(r#"{"prompt":["a"]}"#);
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(prompt_of(&data), &value(r#"["A"]"#));
}

#[tokio::test]
async fn input_empty_array_reply_keeps_the_array() {
    let guardrail = FakeGuardrail::replying(&[]);
    let mut data = body(r#"{"prompt":["a","b"]}"#);
    let untouched = data.clone();
    process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls().len(), 1);
    assert_eq!(data, untouched);
}

#[rstest]
#[case::missing(r#"{"model":"m"}"#)]
#[case::null(r#"{"prompt":null}"#)]
#[tokio::test]
async fn input_missing_or_null_prompt_makes_no_call(#[case] raw: &str) {
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
        written.contains("No prompt found in request data"),
        "{written}"
    );
}

#[rstest]
#[case::integer(r#"{"prompt":7}"#, "int")]
#[case::boolean(r#"{"prompt":true}"#, "bool")]
#[case::float(r#"{"prompt":1.5}"#, "float")]
#[case::object(r#"{"prompt":{"text":"x"}}"#, "dict")]
#[tokio::test]
async fn input_unexpected_prompt_type_warns_and_makes_no_call(
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
        written.contains(&format!("Unexpected prompt type: {kind}")),
        "{written}"
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
        CompletionTranslationError::Guardrail(returned) => assert_eq!(returned, failure),
    }
    assert_eq!(data, untouched);
}

#[tokio::test]
async fn output_rewrites_text_choices_and_leaves_other_choices_alone() {
    let (log, _guard) = capture_logs(tracing::Level::DEBUG);
    let guardrail = FakeGuardrail::replying(&["A", "C"]);
    let mut response = value(
        r#"{"model":"gpt","choices":[{"text":"a"},{"text":null},{"text":"c"},{"finish_reason":"length"}]}"#,
    );
    let original = response.clone();
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        response,
        value(
            r#"{"model":"gpt","choices":[{"text":"A"},{"text":null},{"text":"C"},{"finish_reason":"length"}]}"#
        )
    );
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(
        calls[0].inputs.texts,
        Some(vec!["a".to_owned(), "c".to_owned()])
    );
    assert_eq!(calls[0].inputs.model, Some("gpt".to_owned()));
    assert_eq!(calls[0].request_data.get("response"), Some(&original));
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains("Applied guardrail to choice[0] text. Original length: 1, New length: 1"),
        "{written}"
    );
    assert!(
        written.contains("Applied guardrail to choice[2] text. Original length: 1, New length: 1"),
        "{written}"
    );
}

#[rstest]
#[case::text(r#"{"model":"m","choices":[{"text":"a"}]}"#, Some("m"))]
#[case::empty(r#"{"model":"","choices":[{"text":"a"}]}"#, None)]
#[case::integer(r#"{"model":7,"choices":[{"text":"a"}]}"#, None)]
#[case::null(r#"{"model":null,"choices":[{"text":"a"}]}"#, None)]
#[case::absent(r#"{"choices":[{"text":"a"}]}"#, None)]
#[tokio::test]
async fn output_model_is_sent_only_when_it_is_a_non_empty_text(
    #[case] raw: &str,
    #[case] expected: Option<&str>,
) {
    let guardrail = FakeGuardrail::replying(&["A"]);
    let mut response = value(raw);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls()[0].inputs.model.as_deref(), expected);
}

#[tokio::test]
async fn output_adds_user_metadata_to_the_given_request_data() {
    let guardrail = FakeGuardrail::replying(&["A"]);
    let mut response = value(r#"{"choices":[{"text":"a"}]}"#);
    let user_api_key_dict = body(r#"{"user_id":"u-1","team_id":null,"_internal":"hidden"}"#);
    let mut request_data = body(r#"{"model":"gpt"}"#);
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
        Some(&GuardrailValue::Text("gpt".to_owned()))
    );
    assert!(recorded.contains_key("response"));
}

#[tokio::test]
async fn output_builds_the_request_data_when_none_is_given() {
    let guardrail = FakeGuardrail::replying(&["A"]);
    let mut response = value(r#"{"choices":[{"text":"a"}]}"#);
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
async fn output_keeps_an_existing_response_and_metadata_key() {
    let guardrail = FakeGuardrail::replying(&["A"]);
    let mut response = value(r#"{"choices":[{"text":"a"}]}"#);
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

#[rstest]
#[case::no_choices(r#"{"model":"gpt"}"#)]
#[case::null_choices(r#"{"choices":null}"#)]
#[case::empty_choices(r#"{"choices":[]}"#)]
#[case::choices_without_text(r#"{"choices":[{"text":null},{"finish_reason":"length"}]}"#)]
#[case::not_an_object(r#""text""#)]
#[tokio::test]
async fn output_without_text_makes_no_call_and_leaves_the_request_data(#[case] raw: &str) {
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
}

#[tokio::test]
async fn output_short_reply_leaves_the_later_choices_alone() {
    let guardrail = FakeGuardrail::replying(&["A"]);
    let mut response = value(r#"{"choices":[{"text":"a"},{"text":"b"}]}"#);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        response,
        value(r#"{"choices":[{"text":"A"},{"text":"b"}]}"#)
    );
}

#[tokio::test]
async fn output_long_reply_drops_its_extra_texts() {
    let guardrail = FakeGuardrail::replying(&["A", "B"]);
    let mut response = value(r#"{"choices":[{"text":"a"}]}"#);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(response, value(r#"{"choices":[{"text":"A"}]}"#));
}

#[tokio::test]
async fn output_empty_reply_keeps_the_response() {
    let guardrail = FakeGuardrail::replying(&[]);
    let mut response = value(r#"{"choices":[{"text":"a"}]}"#);
    let untouched = response.clone();
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert_eq!(guardrail.calls().len(), 1);
    assert_eq!(response, untouched);
}

#[tokio::test]
async fn output_guardrail_failure_is_returned_and_leaves_the_response_alone() {
    let failure = GuardrailFailure::Other("blocked by fake".to_owned());
    let guardrail = FakeGuardrail::failing(failure.clone());
    let mut response = value(r#"{"choices":[{"text":"a"}]}"#);
    let untouched = response.clone();
    let error = process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap_err();
    match error {
        CompletionTranslationError::Guardrail(returned) => assert_eq!(returned, failure),
    }
    assert_eq!(response, untouched);
}
