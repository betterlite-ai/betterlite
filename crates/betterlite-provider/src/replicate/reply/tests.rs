#![cfg(test)]

use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::builtin::Message;
use betterlite_tokenizer::count::error::CountError;
use http::HeaderMap;
use rstest::rstest;

use super::{
    PostCall, PostCallLogger, ReplicateReply, ReplyInput, TokenCounter, get_prediction_url,
    transform_response,
};
use crate::replicate::error::ReplicateFailure;
use crate::replicate::request::ReplicateBody;

struct FixedCounter {
    prompt: Option<i64>,
    completion: Option<i64>,
    message_models: Mutex<Vec<String>>,
    responses: Mutex<Vec<(String, String)>>,
}

impl FixedCounter {
    fn new(prompt: Option<i64>, completion: Option<i64>) -> Self {
        Self {
            prompt,
            completion,
            message_models: Mutex::new(Vec::new()),
            responses: Mutex::new(Vec::new()),
        }
    }

    fn message_models(&self) -> Vec<String> {
        self.message_models.lock().unwrap().clone()
    }

    fn responses(&self) -> Vec<(String, String)> {
        self.responses.lock().unwrap().clone()
    }
}

#[async_trait]
impl TokenCounter for FixedCounter {
    async fn count_messages(&self, model: &str, _messages: &[Message]) -> Result<i64, CountError> {
        self.message_models
            .lock()
            .unwrap()
            .push(String::from(model));
        self.prompt.ok_or_else(|| CountError::MissingInput)
    }

    async fn count_response_tokens(&self, model: &str, text: &str) -> Result<i64, CountError> {
        self.responses
            .lock()
            .unwrap()
            .push((String::from(model), String::from(text)));
        self.completion.ok_or_else(|| CountError::MissingInput)
    }
}

#[derive(Default)]
struct RecordingLogger {
    bodies: Mutex<Vec<String>>,
}

impl PostCallLogger for RecordingLogger {
    fn post_call(&self, call: PostCall<'_>) {
        self.bodies
            .lock()
            .unwrap()
            .push(call.original_response.to_owned());
    }
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-a", "b".parse().unwrap());
    headers
}

fn parse_messages(wire: &str) -> Vec<Message> {
    serde_json::from_str(wire).unwrap()
}

async fn transform(
    body: &str,
    messages: &[Message],
    counter: &FixedCounter,
    logger: &RecordingLogger,
) -> Result<ReplicateReply, ReplicateFailure> {
    let request_data = ReplicateBody {
        input: Object::new(),
        version: None,
    };
    let headers = headers();
    let input = ReplyInput {
        model: "m",
        body: body.as_bytes(),
        headers: &headers,
        messages,
        request_data: &request_data,
        api_key: Some("k"),
    };
    transform_response(&input, counter, logger).await
}

fn counter(prompt: i64, completion: i64) -> FixedCounter {
    FixedCounter::new(Some(prompt), Some(completion))
}

fn recorded(model: &str, text: &str) -> (String, String) {
    (String::from(model), String::from(text))
}

#[test]
fn the_prediction_url_is_urls_get() {
    let url =
        get_prediction_url(br#"{"urls":{"get":"https://poll/1"}}"#, &HeaderMap::new()).unwrap();
    assert_eq!(url, "https://poll/1");
}

#[rstest]
#[case::empty_urls(br#"{"urls":{}}"#, "{'urls': {}}")]
#[case::null_get(br#"{"urls":{"get":null}}"#, "{'urls': {'get': None}}")]
#[case::null_urls(br#"{"urls":null}"#, "{'urls': None}")]
#[case::no_urls(b"{}", "{}")]
fn a_missing_prediction_url_fails_400_with_the_reply(#[case] wire: &[u8], #[case] rendered: &str) {
    let headers = headers();
    let error = get_prediction_url(wire, &headers).unwrap_err();
    assert_eq!(status(&error), Some(400));
    assert_eq!(
        message(&error),
        Some(format!("LiteLLM Error - prediction url is None - {rendered}").as_str())
    );
    let ReplicateFailure::Api(replicate) = error else {
        panic!("expected the Replicate error");
    };
    assert_eq!(replicate.headers, Some(headers));
}

#[test]
fn a_prediction_url_that_is_not_a_string_is_rejected() {
    let error = get_prediction_url(br#"{"urls":{"get":3}}"#, &HeaderMap::new()).unwrap_err();
    assert!(matches!(error, ReplicateFailure::PredictionUrlType));
}

#[tokio::test]
async fn a_reply_joins_the_output_and_counts_usage() {
    let logger = RecordingLogger::default();
    let messages = parse_messages(r#"[{"role":"user","content":"hi"}]"#);
    let counter = counter(7, 5);
    let reply = transform(
        r#"{"status":"succeeded","output":["he","llo"]}"#,
        &messages,
        &counter,
        &logger,
    )
    .await
    .unwrap();
    assert_eq!(counter.message_models(), ["m"]);
    assert_eq!(counter.responses(), [recorded("m", "hello")]);
    assert_eq!(reply.model, "replicate/m");
    assert_eq!(reply.text, "hello");
    assert_eq!(reply.usage.prompt_tokens, 7);
    assert_eq!(reply.usage.completion_tokens, 5);
    assert_eq!(reply.usage.total_tokens, 12);
    assert_eq!(reply.created, None);
    assert_eq!(
        *logger.bodies.lock().unwrap(),
        [String::from(
            r#"{"status":"succeeded","output":["he","llo"]}"#
        )]
    );
}

#[rstest]
#[case::empty_list(r#"{"status":"succeeded","output":[]}"#)]
#[case::empty_string(r#"{"status":"succeeded","output":[""]}"#)]
#[case::absent(r#"{"status":"succeeded"}"#)]
#[case::empty_output_string(r#"{"status":"succeeded","output":""}"#)]
#[tokio::test]
async fn an_empty_output_becomes_one_space(#[case] body: &str) {
    let counter = counter(1, 1);
    let reply = transform(
        body,
        &parse_messages("[]"),
        &counter,
        &RecordingLogger::default(),
    )
    .await
    .unwrap();
    assert_eq!(reply.text, " ");
    assert_eq!(counter.responses(), [recorded("m", " ")]);
}

#[tokio::test]
async fn a_string_output_is_its_own_text() {
    let reply = transform(
        r#"{"status":"succeeded","output":"abc"}"#,
        &parse_messages("[]"),
        &counter(1, 3),
        &RecordingLogger::default(),
    )
    .await
    .unwrap();
    assert_eq!(reply.text, "abc");
}

#[tokio::test]
async fn a_null_output_fails_422() {
    let error = transform(
        r#"{"status":"succeeded","output":null}"#,
        &parse_messages("[]"),
        &counter(0, 0),
        &RecordingLogger::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(status(&error), Some(422));
    assert_eq!(message(&error), Some("Unable to parse response. Got=None"));
    assert_eq!(headers_of(&error), Some(headers()));
}

#[tokio::test]
async fn a_reply_that_is_not_an_object_is_rejected() {
    let error = transform(
        "[1]",
        &parse_messages("[]"),
        &counter(0, 0),
        &RecordingLogger::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ReplicateFailure::ReplyNotObject));
}

#[tokio::test]
async fn a_status_other_than_succeeded_fails_422_with_the_reply() {
    let error = transform(
        r#"{"status":"failed","error":"x"}"#,
        &parse_messages("[]"),
        &counter(0, 0),
        &RecordingLogger::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(status(&error), Some(422));
    assert_eq!(
        message(&error),
        Some("LiteLLM Error - prediction not succeeded - {'status': 'failed', 'error': 'x'}")
    );
    assert_eq!(headers_of(&error), Some(headers()));
}

#[tokio::test]
async fn a_missing_status_is_not_succeeded() {
    let error = transform(
        r#"{"output":["a"]}"#,
        &parse_messages("[]"),
        &counter(0, 0),
        &RecordingLogger::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        message(&error),
        Some("LiteLLM Error - prediction not succeeded - {'output': ['a']}")
    );
}

#[rstest]
#[case::prompt(None, Some(1))]
#[case::completion(Some(1), None)]
#[tokio::test]
async fn a_failing_token_counter_fails_the_reply(
    #[case] prompt: Option<i64>,
    #[case] completion: Option<i64>,
) {
    let counter = FixedCounter::new(prompt, completion);
    let error = transform(
        r#"{"status":"succeeded","output":["ok"]}"#,
        &parse_messages("[]"),
        &counter,
        &RecordingLogger::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ReplicateFailure::TokenCount { .. }));
}

#[tokio::test]
async fn an_overflowing_usage_is_an_error() {
    let error = transform(
        r#"{"status":"succeeded","output":["ok"]}"#,
        &parse_messages("[]"),
        &counter(i64::MAX, 1),
        &RecordingLogger::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ReplicateFailure::UsageOverflow));
}

#[tokio::test]
async fn a_reply_that_is_not_json_is_a_typed_error() {
    let error = transform(
        "{not json",
        &parse_messages("[]"),
        &counter(0, 0),
        &RecordingLogger::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ReplicateFailure::Decode { .. }));
}

fn status(failure: &ReplicateFailure) -> Option<u16> {
    failure.api().map(|error| error.status_code)
}

fn headers_of(failure: &ReplicateFailure) -> Option<HeaderMap> {
    failure.api()?.headers.clone()
}

fn message(failure: &ReplicateFailure) -> Option<&str> {
    failure.api().map(|error| error.message.as_str())
}
