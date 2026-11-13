use std::collections::BTreeMap;
use std::fs;

use async_trait::async_trait;
use betterlite_openai::error::ProviderError;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::builtin::Message;
use rstest::rstest;
use serde_json::Number;

use super::super::decode::{DecodedReply, decode_predibase_reply, output_parser};
use super::super::error::PredibaseError;
use super::super::reply::{
    PredibaseResponse, PredibaseUsage, ReplyInput, TokenCounter, transform_response,
};
use super::super::request::RequestBody;
use super::{messages, params};

fn reply_fixture(body: &str) -> Fixture {
    Fixture {
        request: RequestBody {
            inputs: None,
            parameters: Object::new(),
        },
        headers: BTreeMap::new(),
        options: Object::new(),
        messages: Vec::new(),
        body: body.as_bytes().to_vec(),
        status: 200,
        model: String::from("model"),
        created: 1_700_000_000,
    }
}

struct Fixture {
    request: RequestBody,
    headers: BTreeMap<String, String>,
    options: Object,
    messages: Vec<Message>,
    body: Vec<u8>,
    status: u16,
    model: String,
    created: i64,
}

impl Fixture {
    fn input(&self) -> ReplyInput<'_> {
        ReplyInput {
            body: &self.body,
            status: self.status,
            headers: &self.headers,
            request_body: &self.request,
            options: &self.options,
            messages: &self.messages,
            model: &self.model,
            created: self.created,
        }
    }
}

struct Counting {
    fail_messages: bool,
    fail_text: bool,
}

impl Counting {
    const fn working() -> Self {
        Self {
            fail_messages: false,
            fail_text: false,
        }
    }
}

#[async_trait]
impl TokenCounter for Counting {
    async fn count_messages(&self, messages: &[Message]) -> Option<i64> {
        (!self.fail_messages).then(|| i64::try_from(messages.len()).unwrap_or(i64::MAX))
    }

    async fn count_text(&self, text: &str) -> Option<i64> {
        (!self.fail_text).then(|| i64::try_from(text.chars().count()).unwrap_or(i64::MAX))
    }
}

struct FixedText {
    count: i64,
}

#[async_trait]
impl TokenCounter for FixedText {
    async fn count_messages(&self, _messages: &[Message]) -> Option<i64> {
        Some(0)
    }

    async fn count_text(&self, _text: &str) -> Option<i64> {
        Some(self.count)
    }
}

#[tokio::test]
async fn a_reply_gives_content_finish_reason_created_and_model() {
    let fixture = reply_fixture(
        r#"{"generated_text":"<|assistant|>hi","details":{"finish_reason":"length","tokens":[{"logprob":-0.5},{"logprob":null},{"logprob":-0.25}]}}"#,
    );
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.model, "model");
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.choices.len(), 1_usize);
    let choice = &response.choices[0];
    assert_eq!(choice.index, 0);
    assert_eq!(choice.content.as_deref(), Some("hi"));
    assert_eq!(choice.finish_reason, "length");
    assert_eq!(
        choice.private_logprob,
        Some(Number::from_f64(-0.75).unwrap())
    );
    assert_eq!(choice.logprobs, None);
}

#[tokio::test]
async fn integer_logprobs_sum_exactly_with_nulls_skipped() {
    let fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[{"logprob":-3},{"logprob":null},{"logprob":6}]}}"#,
    );
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(
        response.choices[0].private_logprob,
        Some(Number::from(3_i64))
    );
}

#[tokio::test]
async fn integer_logprobs_above_the_signed_range_keep_their_value() {
    let fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[{"logprob":18446744073709551615},{"logprob":1}]}}"#,
    );
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    let expected = Number::from_i128(i128::from(u64::MAX) + 1).unwrap();
    assert_eq!(response.choices[0].private_logprob, Some(expected));
}

#[tokio::test]
async fn an_empty_generated_text_leaves_the_content_null() {
    let fixture = reply_fixture(r#"{"generated_text":""}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices[0].content, None);
    assert_eq!(response.choices[0].finish_reason, "stop");
}

#[tokio::test]
async fn the_finish_reason_maps_through_litellms_table() {
    let fixture = reply_fixture(
        r#"{"generated_text":"x","details":{"finish_reason":"content_filtered","tokens":[]}}"#,
    );
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices[0].finish_reason, "content_filter");
    assert_eq!(
        response.choices[0].private_logprob,
        Some(Number::from(0_i64))
    );
}

#[tokio::test]
async fn an_unmapped_finish_reason_becomes_stop() {
    let fixture =
        reply_fixture(r#"{"generated_text":"x","details":{"finish_reason":"weird","tokens":[]}}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices[0].finish_reason, "stop");
}

#[tokio::test]
async fn a_null_finish_reason_becomes_stop() {
    let fixture =
        reply_fixture(r#"{"generated_text":"x","details":{"finish_reason":null,"tokens":[]}}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices[0].finish_reason, "stop");
}

#[tokio::test]
async fn details_without_tokens_leave_the_finish_reason_alone() {
    let fixture = reply_fixture(r#"{"generated_text":"x","details":{"finish_reason":"length"}}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices[0].finish_reason, "stop");
    assert_eq!(response.choices[0].private_logprob, None);
}

#[tokio::test]
async fn tokens_without_a_finish_reason_fail_with_a_server_error() {
    let fixture = reply_fixture(r#"{"generated_text":"x","details":{"tokens":[]}}"#);
    let error = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains("finish_reason"));
}

#[tokio::test]
async fn best_of_sequences_become_extra_choices() {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"<|user|>b","finish_reason":"length","tokens":[{"logprob":-0.5},{"logprob":null}]},{"generated_text":"","finish_reason":"stop","tokens":[{"logprob":-0.25}]},{"generated_text":"c","finish_reason":"weird","tokens":[]}]}}"#,
    );
    fixture.options = params(r#"{"best_of":2}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 4_usize);
    assert_eq!(response.choices[1].index, 1);
    assert_eq!(response.choices[1].content.as_deref(), Some("b"));
    assert_eq!(response.choices[1].finish_reason, "length");
    assert_eq!(
        response.choices[1].logprobs,
        Some(Number::from_f64(-0.5).unwrap())
    );
    assert_eq!(response.choices[1].private_logprob, None);
    assert_eq!(response.choices[2].index, 2);
    assert_eq!(response.choices[2].content, None);
    assert_eq!(response.choices[2].logprobs, None);
    assert_eq!(response.choices[3].index, 3);
    assert_eq!(response.choices[3].content.as_deref(), Some("c"));
    assert_eq!(response.choices[3].finish_reason, "stop");
    assert_eq!(response.choices[3].logprobs, Some(Number::from(0_i64)));
}

#[tokio::test]
async fn best_of_reads_the_request_parameters_when_the_options_have_none() {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    fixture.request.parameters = params(r#"{"best_of":2}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 2_usize);
}

#[tokio::test]
async fn a_null_best_of_falls_back_to_the_request_parameters() {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    fixture.options = params(r#"{"best_of":null}"#);
    fixture.request.parameters = params(r#"{"best_of":2}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 2_usize);
}

#[tokio::test]
async fn the_options_best_of_wins_over_the_request_parameters() {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    fixture.options = params(r#"{"best_of":1}"#);
    fixture.request.parameters = params(r#"{"best_of":2}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 1_usize);
}

#[rstest]
#[case::string(r#"{"best_of":"2"}"#)]
#[case::float(r#"{"best_of":2.5}"#)]
#[case::boolean(r#"{"best_of":true}"#)]
#[case::null(r#"{"best_of":null}"#)]
#[tokio::test]
async fn a_best_of_that_is_not_an_integer_adds_no_choices(#[case] options: &str) {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    fixture.options = params(options);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 1_usize);
}

#[tokio::test]
async fn a_best_of_above_the_signed_range_still_adds_choices() {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    fixture.options = params(r#"{"best_of":18446744073709551615}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 2_usize);
}

#[tokio::test]
async fn a_reply_with_sequences_alone_adds_no_choices() {
    let fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[]}]}}"#,
    );
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 1_usize);
}

#[tokio::test]
async fn usage_counts_the_messages_and_the_parsed_text() {
    let mut fixture = reply_fixture(r#"{"generated_text":"<|assistant|>hi"}"#);
    fixture.messages = messages(r#"[{"role":"user","content":"hello"}]"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(
        response.usage,
        PredibaseUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3
        }
    );
}

#[tokio::test]
async fn usage_is_zero_when_the_text_counter_fails() {
    let mut fixture = reply_fixture(r#"{"generated_text":"hi"}"#);
    fixture.messages = messages(r#"[{"role":"user","content":"hello"}]"#);
    let failing_text = Counting {
        fail_messages: false,
        fail_text: true,
    };
    let response = transform_response(&fixture.input(), &failing_text)
        .await
        .unwrap();
    assert_eq!(
        response.usage,
        PredibaseUsage {
            prompt_tokens: 1,
            completion_tokens: 0,
            total_tokens: 1
        }
    );
}

#[tokio::test]
async fn usage_is_zero_when_the_message_counter_fails() {
    let mut fixture = reply_fixture(r#"{"generated_text":"hi"}"#);
    fixture.messages = messages(r#"[{"role":"user","content":"hello"}]"#);
    let failing_messages = Counting {
        fail_messages: true,
        fail_text: false,
    };
    let response = transform_response(&fixture.input(), &failing_messages)
        .await
        .unwrap();
    assert_eq!(
        response.usage,
        PredibaseUsage {
            prompt_tokens: 0,
            completion_tokens: 2,
            total_tokens: 2
        }
    );
}

#[tokio::test]
async fn usage_is_zero_for_empty_content() {
    let empty = reply_fixture(r#"{"generated_text":""}"#);
    let response = transform_response(&empty.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(
        response.usage,
        PredibaseUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0
        }
    );
}

#[tokio::test]
async fn an_empty_parsed_text_is_not_counted() {
    let stripped = reply_fixture(r#"{"generated_text":"<s>"}"#);
    let response = transform_response(&stripped.input(), &FixedText { count: 7 })
        .await
        .unwrap();
    assert_eq!(response.choices[0].content.as_deref(), Some(""));
    assert_eq!(response.usage.completion_tokens, 0);
    let text = reply_fixture(r#"{"generated_text":"x"}"#);
    let counted = transform_response(&text.input(), &FixedText { count: 7 })
        .await
        .unwrap();
    assert_eq!(counted.usage.completion_tokens, 7);
}

#[tokio::test]
async fn usage_counts_only_the_first_choice() {
    let mut fixture = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":[{"generated_text":"bbb","finish_reason":"stop","tokens":[]}]}}"#,
    );
    fixture.options = params(r#"{"best_of":2}"#);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(response.choices.len(), 2_usize);
    assert_eq!(response.usage.completion_tokens, 1);
    assert_eq!(response.usage.total_tokens, 1);
}

#[tokio::test]
async fn x_headers_are_copied_with_the_provider_prefix() {
    let mut fixture = reply_fixture(r#"{"generated_text":"a"}"#);
    fixture.headers = BTreeMap::from([
        (String::from("X-Upper"), String::from("u")),
        (String::from("foo"), String::from("f")),
        (String::from("x-request-id"), String::from("r")),
    ]);
    let response = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(
        response.additional_headers,
        BTreeMap::from([
            (String::from("llm_provider-x-request-id"), String::from("r")),
            (String::from("llm_provider-x-upper"), String::from("u")),
        ])
    );
}

#[tokio::test]
async fn an_error_key_raises_the_response_status() {
    let mut fixture = reply_fixture(r#"{"error":"boom"}"#);
    fixture.status = 200;
    let error = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 200);
    assert_eq!(error.message, "boom");
    fixture.status = 502;
    let failed = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(failed.status_code, 502);
}

#[tokio::test]
async fn an_error_object_keeps_its_json_text() {
    let fixture = reply_fixture(r#"{"error":{"message":"boom"}}"#);
    let error = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 200);
    assert!(error.message.contains("boom"));
}

#[tokio::test]
async fn a_null_error_is_still_an_error() {
    let fixture = reply_fixture(r#"{"error":null}"#);
    let error = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(error.status_code, 200);
}

#[rstest]
#[case::not_json("not json", 422)]
#[case::array("[1,2]", 422)]
#[case::string("\"hi\"", 422)]
#[case::string_that_contains_error(r#""error""#, 422)]
#[case::missing_generated_text(r#"{"foo":1}"#, 422)]
#[case::empty_object("{}", 422)]
#[case::null_generated_text(r#"{"generated_text":null}"#, 500)]
#[case::number_generated_text(r#"{"generated_text":5}"#, 500)]
#[case::array_generated_text(r#"{"generated_text":[]}"#, 500)]
#[case::wrong_details(r#"{"generated_text":"x","details":[]}"#, 500)]
#[case::details_array_of_fields(r#"{"generated_text":"x","details":["stop"]}"#, 500)]
#[case::wrong_tokens(r#"{"generated_text":"x","details":{"tokens":5}}"#, 500)]
#[case::token_without_logprob(
    r#"{"generated_text":"x","details":{"finish_reason":"stop","tokens":[{}]}}"#,
    500
)]
#[case::token_entry_not_an_object(
    r#"{"generated_text":"x","details":{"finish_reason":"stop","tokens":[[7]]}}"#,
    500
)]
#[case::string_logprob(
    r#"{"generated_text":"x","details":{"finish_reason":"stop","tokens":[{"logprob":"x"}]}}"#,
    500
)]
#[case::sequence_without_tokens(r#"{"generated_text":"x","details":{"finish_reason":"stop","best_of_sequences":[{"generated_text":"b","finish_reason":"stop"}]}}"#, 500)]
#[case::sequence_entry_not_an_object(
    r#"{"generated_text":"x","details":{"finish_reason":"stop","best_of_sequences":[["b","stop",[]]]}}"#,
    500
)]
#[case::sequence_tokens_entry_not_an_object(
    r#"{"generated_text":"x","details":{"finish_reason":"stop","best_of_sequences":[{"generated_text":"b","finish_reason":"stop","tokens":[[7]]}]}}"#,
    500
)]
#[case::float_sum_not_finite(
    r#"{"generated_text":"x","details":{"finish_reason":"stop","tokens":[{"logprob":1e308},{"logprob":1e308}]}}"#,
    500
)]
#[tokio::test]
async fn bad_replies_give_a_predibase_error(#[case] body: &str, #[case] status: u16) {
    let fixture = reply_fixture(body);
    let error = transform_response(&fixture.input(), &Counting::working())
        .await
        .unwrap_err();
    assert_eq!(error.status_code, status);
}

#[tokio::test]
async fn null_details_and_token_containers_count_as_absent() {
    let details = reply_fixture(r#"{"generated_text":"x","details":null}"#);
    let from_details = transform_response(&details.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(from_details.choices[0].finish_reason, "stop");
    assert_eq!(from_details.choices[0].private_logprob, None);
    let tokens = reply_fixture(
        r#"{"generated_text":"x","details":{"finish_reason":"length","tokens":null}}"#,
    );
    let from_tokens = transform_response(&tokens.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(from_tokens.choices[0].finish_reason, "stop");
    let mut sequences = reply_fixture(
        r#"{"generated_text":"a","details":{"finish_reason":"stop","tokens":[],"best_of_sequences":null}}"#,
    );
    sequences.options = params(r#"{"best_of":2}"#);
    let from_sequences = transform_response(&sequences.input(), &Counting::working())
        .await
        .unwrap();
    assert_eq!(from_sequences.choices.len(), 1_usize);
}

#[tokio::test]
async fn a_wrongly_shaped_details_container_names_the_field() {
    let details = reply_fixture(r#"{"generated_text":"x","details":[]}"#);
    let details_error = transform_response(&details.input(), &Counting::working())
        .await
        .unwrap_err();
    assert!(details_error.message.contains("details"));
    let tokens = reply_fixture(
        r#"{"generated_text":"x","details":{"finish_reason":"stop","tokens":[[7]]}}"#,
    );
    let tokens_error = transform_response(&tokens.input(), &Counting::working())
        .await
        .unwrap_err();
    assert!(tokens_error.message.contains("tokens"));
    let sequences = reply_fixture(
        r#"{"generated_text":"x","details":{"finish_reason":"stop","best_of_sequences":[["b","stop",[]]]}}"#,
    );
    let sequences_error = transform_response(&sequences.input(), &Counting::working())
        .await
        .unwrap_err();
    assert!(sequences_error.message.contains("best_of_sequences"));
}

#[test]
fn the_reply_decoder_is_typed() {
    let decoded = decode_predibase_reply(
        br#"{"generated_text":"hi","details":{"finish_reason":"stop","tokens":[{"logprob":1}]}}"#,
    )
    .unwrap();
    let DecodedReply::Text(text) = decoded else {
        panic!("a text reply");
    };
    assert_eq!(text.generated_text, "hi");
    let details = text.details.unwrap();
    assert_eq!(details.finish_reason, Some(Some(String::from("stop"))));
    assert_eq!(details.tokens.map(|tokens| tokens.len()), Some(1_usize));
}

#[test]
fn a_body_that_is_not_utf8_names_its_size() {
    let error = decode_predibase_reply(&[0xff_u8, 0xfe]).unwrap_err();
    assert_eq!(error.status_code, 422);
    assert!(error.message.contains("2 bytes that are not UTF-8"));
}

#[test]
fn a_malformed_body_keeps_its_text() {
    let body = "not json";
    let error = decode_predibase_reply(body.as_bytes()).unwrap_err();
    assert_eq!(error.status_code, 422);
    assert_eq!(error.message, body);
}

#[test]
fn a_missing_generated_text_names_the_body() {
    let body = r#"{"foo":1}"#;
    let error = decode_predibase_reply(body.as_bytes()).unwrap_err();
    assert_eq!(error.status_code, 422);
    assert!(error.message.contains(body));
}

#[rstest]
#[case::plain("hello", "hello")]
#[case::both_ends("<|assistant|>hi<|assistant|>", "hi")]
#[case::leading_space_kept("  <|assistant|>hi", "  hi")]
#[case::bos_eos("<s>hello</s>", "hello")]
#[case::order_matters("<s><|user|>hi</s>", "<|user|>hi")]
#[case::end_only("abc<s>", "abc")]
#[case::trailing_space_kept("abc </s>", "abc ")]
#[case::system_then_user("<|system|> <|user|>x", " x")]
#[case::empty("", "")]
#[case::blank("   ", "   ")]
#[case::one_of_each("<s><s>x</s></s>", "<s>x</s>")]
#[case::end_of_another_token("<|assistant|>x<s>", "x")]
fn output_parser_strips_template_tokens_in_order(#[case] given: &str, #[case] expected: &str) {
    assert_eq!(output_parser(given), expected);
}

#[test]
fn an_empty_error_key_value_is_reported_as_null() {
    let decoded = decode_predibase_reply(br#"{"error":null,"generated_text":"x"}"#).unwrap();
    assert_eq!(decoded, DecodedReply::Error(String::from("null")));
}

#[test]
fn an_error_key_wins_over_the_generated_text() {
    let decoded = decode_predibase_reply(br#"{"error":"boom","generated_text":"x"}"#).unwrap();
    assert_eq!(decoded, DecodedReply::Error(String::from("boom")));
}

#[test]
fn replays_the_predibase_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/predibase_reply")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let _replayed = decode_predibase_reply(&bytes);
        })
        .count();
    assert!(seen > 0_usize);
}

#[test]
fn a_predibase_error_converts_to_the_gateway_error() {
    let error = PredibaseError {
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
fn a_predibase_response_is_debuggable() {
    let response = PredibaseResponse {
        model: String::from("m"),
        created: 1,
        choices: Vec::new(),
        usage: PredibaseUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        },
        additional_headers: BTreeMap::new(),
    };
    assert!(!format!("{response:?}").is_empty());
}
