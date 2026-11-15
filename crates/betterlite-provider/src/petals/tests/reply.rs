use std::collections::BTreeMap;
use std::fs;

use async_trait::async_trait;
use betterlite_openai::error::ProviderError;
use rstest::rstest;

use super::super::decode::decode_petals_reply;
use super::super::error::PetalsError;
use super::super::reply::{PetalsResponse, ReplyInput, TokenCounter, transform_response};

struct Fixture {
    headers: BTreeMap<String, String>,
    prompt: Option<String>,
    body: Vec<u8>,
    status: u16,
    model: String,
    created: i64,
}

fn reply_fixture(body: &str) -> Fixture {
    Fixture {
        headers: BTreeMap::new(),
        prompt: Some(String::from("<s>[INST] hi [/INST]\n")),
        body: body.as_bytes().to_vec(),
        status: 200,
        model: String::from("meta-llama/llama-2-7b-chat"),
        created: 1_700_000_000,
    }
}

impl Fixture {
    fn input(&self) -> ReplyInput<'_> {
        ReplyInput {
            body: &self.body,
            status: self.status,
            headers: &self.headers,
            prompt: self.prompt.as_deref(),
            model: &self.model,
            created: self.created,
        }
    }
}

struct StandInCounter {
    fail_text: bool,
}

#[async_trait]
impl TokenCounter for StandInCounter {
    async fn count_text(&self, text: &str) -> Option<i64> {
        (!self.fail_text).then(|| i64::try_from(text.chars().count()).unwrap_or(i64::MAX))
    }
}

fn counter() -> StandInCounter {
    StandInCounter { fail_text: false }
}

async fn response(fixture: &Fixture) -> Result<PetalsResponse, PetalsError> {
    transform_response(&fixture.input(), &counter()).await
}

#[test]
fn a_string_output_decodes() {
    let reply = decode_petals_reply(br#"{"outputs":"hello"}"#).unwrap();
    assert_eq!(reply.outputs, "hello");
}

#[test]
fn other_fields_are_ignored() {
    let reply = decode_petals_reply(br#"{"outputs":"hi","detail":"x"}"#).unwrap();
    assert_eq!(reply.outputs, "hi");
}

#[test]
fn a_blank_output_is_still_a_reply() {
    let reply = decode_petals_reply(br#"{"outputs":" "}"#).unwrap();
    assert_eq!(reply.outputs, " ");
}

#[rstest]
#[case::not_json("not json", "the petals reply is not JSON")]
#[case::empty_body("", "the petals reply is not JSON")]
#[case::truncated(r#"{"outputs":"hel"#, "the petals reply is not JSON")]
#[case::array("[1,2]", "the petals reply is not a JSON object")]
#[case::text(r#""hi""#, "the petals reply is not a JSON object")]
#[case::null("null", "the petals reply is not a JSON object")]
#[case::missing(r#"{"detail":"nope"}"#, "the petals reply has no outputs field")]
#[case::empty_object("{}", "the petals reply has no outputs field")]
#[case::null_outputs(r#"{"outputs":null}"#, "the petals reply outputs is not a string")]
#[case::number_outputs(r#"{"outputs":5}"#, "the petals reply outputs is not a string")]
#[case::list_outputs(r#"{"outputs":["a"]}"#, "the petals reply outputs is not a string")]
#[case::object_outputs(r#"{"outputs":{}}"#, "the petals reply outputs is not a string")]
#[case::empty_outputs(r#"{"outputs":""}"#, "the petals reply outputs is empty")]
fn malformed_replies_are_rejected(#[case] body: &str, #[case] message: &str) {
    let error = decode_petals_reply(body.as_bytes()).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[tokio::test]
async fn a_string_output_is_the_reply() {
    let response = response(&reply_fixture(r#"{"outputs":"hello"}"#))
        .await
        .unwrap();
    assert_eq!(response.model, "meta-llama/llama-2-7b-chat");
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].index, 0);
    assert_eq!(response.choices[0].finish_reason, "stop");
    assert_eq!(response.choices[0].content, "hello");
    assert_eq!(response.usage.prompt_tokens, 21);
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(response.usage.total_tokens, 26);
}

#[tokio::test]
async fn a_unicode_output_is_counted_in_characters() {
    let response = response(&reply_fixture(r#"{"outputs":"héllo"}"#))
        .await
        .unwrap();
    assert_eq!(response.choices[0].content, "héllo");
    assert_eq!(response.usage.completion_tokens, 5);
}

#[tokio::test]
async fn a_created_status_is_read() {
    let mut fixture = reply_fixture(r#"{"outputs":"hello"}"#);
    fixture.status = 201;
    let response = response(&fixture).await.unwrap();
    assert_eq!(response.choices[0].content, "hello");
}

#[tokio::test]
async fn failing_counters_count_zero() {
    let fixture = reply_fixture(r#"{"outputs":"hello"}"#);
    let counter = StandInCounter { fail_text: true };
    let response = transform_response(&fixture.input(), &counter)
        .await
        .unwrap();
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 0);
    assert_eq!(response.usage.total_tokens, 0);
}

#[tokio::test]
async fn a_missing_prompt_is_a_500() {
    let mut fixture = reply_fixture(r#"{"outputs":"hello"}"#);
    fixture.prompt = None;
    let error = response(&fixture).await.unwrap_err();
    assert_eq!(error.status_code, 500);
    assert_eq!(
        error.message,
        "the petals prompt is missing, so the reply has no prompt to count"
    );
}

#[rstest]
#[case::bad_request(400)]
#[case::not_found(404)]
#[case::teapot(418)]
#[case::server_error(500)]
#[tokio::test]
async fn an_error_status_keeps_the_upstream_status(#[case] status: u16) {
    let mut fixture = reply_fixture(r#"{"outputs":"hello"}"#);
    fixture.status = status;
    fixture
        .headers
        .insert(String::from("x-a"), String::from("1"));
    let error = response(&fixture).await.unwrap_err();
    assert_eq!(error.status_code, status);
    assert_eq!(error.message, r#"PetalsException - {"outputs":"hello"}"#);
    assert_eq!(error.headers.get("x-a").map(String::as_str), Some("1"));
}

#[tokio::test]
async fn a_redirect_status_is_an_apiconnection_error() {
    let mut fixture = reply_fixture(r#"{"outputs":"hello"}"#);
    fixture.status = 302;
    let error = response(&fixture).await.unwrap_err();
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, r#"PetalsException - {"outputs":"hello"}"#);
}

#[tokio::test]
async fn an_error_body_that_is_not_utf8_is_described_by_its_length() {
    let mut fixture = reply_fixture("");
    fixture.body = vec![0xff_u8, 0xfe];
    fixture.status = 500;
    let error = response(&fixture).await.unwrap_err();
    assert_eq!(
        error.message,
        "PetalsException - 2 bytes that are not UTF-8"
    );
}

#[rstest]
#[case::malformed(200)]
#[case::no_content(204)]
#[tokio::test]
async fn an_unusable_body_is_a_500(#[case] status: u16) {
    let mut fixture = reply_fixture(r#"{"detail":"nope"}"#);
    fixture.status = status;
    if status == 204 {
        fixture.body.clear();
    }
    let error = response(&fixture).await.unwrap_err();
    assert_eq!(error.status_code, 500);
}

#[test]
fn replays_the_petals_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/petals_reply")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let _replayed = decode_petals_reply(&bytes);
        })
        .count();
    assert!(seen > 0_usize);
}

#[test]
fn a_petals_error_converts_to_the_gateway_error() {
    let error = PetalsError {
        status_code: 418,
        message: String::from("boom"),
        headers: BTreeMap::from([(String::from("x"), String::from("1"))]),
    };
    let converted = ProviderError::from(error);
    assert_eq!(converted.status_code, 418);
    assert_eq!(converted.message, "boom");
    assert_eq!(
        converted.headers,
        Some(vec![(String::from("x"), String::from("1"))])
    );
}

#[test]
fn a_petals_error_without_headers_converts_without_them() {
    let error = PetalsError {
        status_code: 500,
        message: String::from("boom"),
        headers: BTreeMap::new(),
    };
    let converted = ProviderError::from(error);
    assert_eq!(converted.headers, None);
}
